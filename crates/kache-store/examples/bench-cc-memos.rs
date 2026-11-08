use anyhow::{Context, Result, ensure};
use kache_store::file_hash::{CcPreprocessMemoInput, FileFingerprint, FileHashCache};
use deser::{Deserialize, Serialize};
use std::{
    hint::black_box,
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Clone, Deserialize, Serialize)]
struct Sample {
    memo_key: String,
    preprocessed_hash: String,
    inputs: Vec<CcPreprocessMemoInput>,
}

#[derive(Serialize)]
struct Measurement {
    phase: &'static str,
    trial: usize,
    elapsed_ms: f64,
    ops_per_sec: f64,
}

#[derive(Clone, Copy)]
enum Tier {
    Small,
    Typical,
    Heavy,
}

impl Tier {
    fn from_name(name: &str) -> Result<Self> {
        match name {
            "small" => Ok(Self::Small),
            "typical" => Ok(Self::Typical),
            "heavy" => Ok(Self::Heavy),
            other => anyhow::bail!("unknown tier: {other}; expected small, typical, or heavy"),
        }
    }

    fn params(self) -> (usize, usize, usize) {
        match self {
            Self::Small => (50, 120, 18),
            Self::Typical => (500, 900, 45),
            Self::Heavy => (2500, 4500, 85),
        }
    }
}

fn elapsed(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn generate_fixtures(tier: Tier) -> Vec<Sample> {
    let (tu_count, header_pool_size, avg_headers_per_tu) = tier.params();
    let universal_count = (header_pool_size / 20).max(4);
    let module_count = (tu_count / 15).max(3);

    let headers: Vec<(String, FileFingerprint, String)> = (0..header_pool_size)
        .map(|idx| {
            let path = if idx < universal_count {
                format!("sys/kache_core_{idx}.h")
            } else if idx < universal_count + (header_pool_size / 4) {
                let mod_idx = idx % module_count;
                format!("include/module_{mod_idx}/types_{idx}.h")
            } else {
                let mod_idx = idx % module_count;
                format!("src/module_{mod_idx}/detail_{idx}.h")
            };
            let content_hash = blake3::hash(format!("content-bytes-header-{idx}").as_bytes())
                .to_hex()
                .to_string();
            let fingerprint = FileFingerprint {
                path: format!("/checkout/{path}"),
                size: (128 + (idx % 4096)) as i64,
                mtime_ns: 1_700_000_000_000_000_000 + (idx as i64 * 10_000),
                ctime_ns: 1_700_000_000_000_000_000 + (idx as i64 * 10_000),
                inode: 1_000_000 + idx as i64,
            };
            (path, fingerprint, content_hash)
        })
        .collect();

    let mut samples = Vec::with_capacity(tu_count);
    for tu_idx in 0..tu_count {
        let _mod_idx = tu_idx % module_count;
        let mut tu_inputs = Vec::with_capacity(avg_headers_per_tu);

        for (path, fp, ch) in headers.iter().take(universal_count) {
            tu_inputs.push(CcPreprocessMemoInput {
                name: path.clone(),
                fingerprint: fp.clone(),
                content: ch.clone(),
                mapped: format!("mapped-{ch}"),
                observed_ns: 20_000_000_000,
            });
        }

        let module_headers_start = universal_count;
        let module_headers_end = universal_count + (header_pool_size / 4);
        for (idx, (path, fp, ch)) in headers[module_headers_start..module_headers_end]
            .iter()
            .enumerate()
        {
            if (idx + tu_idx) % module_count == 0 && tu_inputs.len() < avg_headers_per_tu / 2 {
                tu_inputs.push(CcPreprocessMemoInput {
                    name: path.clone(),
                    fingerprint: fp.clone(),
                    content: ch.clone(),
                    mapped: format!("mapped-{ch}"),
                    observed_ns: 20_000_000_000,
                });
            }
        }

        let leaf_start = module_headers_end;
        let leaf_count = headers.len() - leaf_start;
        let mut pick = (tu_idx * 17) % leaf_count;
        while tu_inputs.len() < avg_headers_per_tu {
            let (path, fp, ch) = &headers[leaf_start + (pick % leaf_count)];
            tu_inputs.push(CcPreprocessMemoInput {
                name: path.clone(),
                fingerprint: fp.clone(),
                content: ch.clone(),
                mapped: format!("mapped-{ch}"),
                observed_ns: 20_000_000_000,
            });
            pick += 1;
        }

        let memo_key = blake3::hash(format!("tu-memo-key-flags-compile-unit-{tu_idx}").as_bytes())
            .to_hex()
            .to_string();
        let preprocessed_hash =
            blake3::hash(format!("tu-preprocessed-tokens-hash-{tu_idx}").as_bytes())
                .to_hex()
                .to_string();

        samples.push(Sample {
            memo_key,
            preprocessed_hash,
            inputs: tu_inputs,
        });
    }

    samples
}

fn run_benchmark(
    samples: &[Sample],
    output_dir: &Path,
    trials: usize,
) -> Result<deser_value::Value> {
    std::fs::create_dir_all(output_dir)?;
    let db_path = output_dir.join("shared_memos.db");
    if db_path.exists() {
        let _ = std::fs::remove_file(&db_path);
    }
    let wal_path = output_dir.join("shared_memos.db-wal");
    if wal_path.exists() {
        let _ = std::fs::remove_file(&wal_path);
    }

    let shared = FileHashCache::open(&db_path)?;
    let mut measurements = Vec::new();

    let write_start = Instant::now();
    for sample in samples {
        shared.put_cc_preprocess_memo_inputs(
            &sample.memo_key,
            &sample.preprocessed_hash,
            &sample.inputs,
        )?;
    }
    let write_ms = elapsed(write_start);
    measurements.push(Measurement {
        phase: "write_batch",
        trial: 0,
        elapsed_ms: write_ms,
        ops_per_sec: (samples.len() as f64) / (write_ms / 1000.0),
    });

    for trial in 0..trials {
        let read_start = Instant::now();
        let forward = trial % 2 == 0;
        let iter: Box<dyn Iterator<Item = &Sample>> = if forward {
            Box::new(samples.iter())
        } else {
            Box::new(samples.iter().rev())
        };

        for sample in iter {
            let memo = shared
                .get_cc_preprocess_memo(&sample.memo_key)?
                .context("missing stored memo")?;
            ensure!(
                memo.preprocessed_hash == sample.preprocessed_hash
                    && memo.inputs.len() == sample.inputs.len(),
                "stored memo content mismatch"
            );
            black_box(memo.inputs);
        }
        let read_ms = elapsed(read_start);
        measurements.push(Measurement {
            phase: "read_hot_lookup",
            trial,
            elapsed_ms: read_ms,
            ops_per_sec: (samples.len() as f64) / (read_ms / 1000.0),
        });
    }

    let touch_start = Instant::now();
    for sample in samples {
        shared.touch_cc_preprocess_memo(&sample.memo_key)?;
    }
    let touch_ms = elapsed(touch_start);
    measurements.push(Measurement {
        phase: "touch_memos",
        trial: 0,
        elapsed_ms: touch_ms,
        ops_per_sec: (samples.len() as f64) / (touch_ms / 1000.0),
    });

    let prune_start = Instant::now();
    let (pruned_memos, pruned_inputs) = shared.prune_cc_preprocess_memos()?;
    let prune_ms = elapsed(prune_start);
    measurements.push(Measurement {
        phase: "prune_stale",
        trial: 0,
        elapsed_ms: prune_ms,
        ops_per_sec: (samples.len() as f64) / (prune_ms / 1000.0),
    });

    shared
        .db()
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;

    let unique_inputs: i64 = shared
        .db()
        .query_row("SELECT count(*) FROM cc_memo_inputs", [], |r| r.get(0))?;
    let stored_memos: i64 = shared
        .db()
        .query_row("SELECT count(*) FROM cc_preprocess_memos", [], |r| {
            r.get(0)
        })?;
    let total_refs: usize = samples.iter().map(|s| s.inputs.len()).sum();
    let db_bytes = std::fs::metadata(&db_path)?.len();
    let wal_bytes = std::fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0);

    let report = deser_value::value!({
        "memos_evaluated": samples.len(),
        "stored_memos": stored_memos,
        "input_references": total_refs,
        "unique_inputs": unique_inputs,
        "deduplication_ratio": (total_refs as f64) / (unique_inputs.max(1) as f64),
        "db_file_bytes": db_bytes,
        "wal_file_bytes": wal_bytes,
        "pruned_memos": pruned_memos,
        "pruned_inputs": pruned_inputs,
        "measurements": deser_value::to_value(&measurements)?,
    });

    let report_str = deser_json::SerializerConfig::builder()
        .pretty(deser_json::Indent::Spaces(2))
        .build()
        .to_string(&report)?;
    std::fs::write(output_dir.join("report.json"), &report_str)?;
    println!("{report_str}");

    Ok(report)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut tier = Tier::Typical;
    let mut output_dir: Option<PathBuf> = None;
    let mut sample_path: Option<PathBuf> = None;
    let mut trials = 4;

    let mut idx = 1;
    while idx < args.len() {
        match args[idx].as_str() {
            "--tier" => {
                idx += 1;
                ensure!(idx < args.len(), "--tier requires an argument");
                tier = Tier::from_name(&args[idx])?;
            }
            "--output" => {
                idx += 1;
                ensure!(idx < args.len(), "--output requires a path");
                output_dir = Some(PathBuf::from(&args[idx]));
            }
            "--sample" => {
                idx += 1;
                ensure!(idx < args.len(), "--sample requires a JSON path");
                sample_path = Some(PathBuf::from(&args[idx]));
            }
            "--trials" => {
                idx += 1;
                ensure!(idx < args.len(), "--trials requires an integer");
                trials = args[idx].parse::<usize>()?;
            }
            "--help" | "-h" => {
                println!(
                    "usage: bench-cc-memos [--tier small|typical|heavy] [--output DIR] [--sample FILE.json] [--trials N]"
                );
                return Ok(());
            }
            other => {
                if idx == 1 && !other.starts_with('-') && args.len() == 3 {
                    sample_path = Some(PathBuf::from(other));
                    output_dir = Some(PathBuf::from(&args[2]));
                    break;
                }
                anyhow::bail!("unknown option: {other}");
            }
        }
        idx += 1;
    }

    let samples = if let Some(path) = sample_path {
        let raw = std::fs::read(&path)
            .with_context(|| format!("failed to read sample from {}", path.display()))?;
        let parsed: Vec<Sample> = deser_json::from_slice(&raw)?;
        ensure!(!parsed.is_empty(), "sample must not be empty");
        parsed
    } else {
        generate_fixtures(tier)
    };

    let target_dir = match output_dir {
        Some(dir) => dir,
        None => {
            let tmp = PathBuf::from(".work/tmp").join(format!("bench-cc-memos-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&tmp);
            tmp
        }
    };

    run_benchmark(&samples, &target_dir, trials)?;
    Ok(())
}
