use super::{Config, INVOCATION_SETTINGS, Store, open_primary_and_fallback_for};
use crate::invocation::InvocationEnv;
use crate::store::{StoreOptions, StoreReuseStamp, VerifyRestores};
use anyhow::Result;
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

type Stores = (Store, Vec<Store>);

#[derive(PartialEq, Eq)]
struct Key {
    directory: PathBuf,
    config: String,
}

struct Idle {
    key: Key,
    stamps: Vec<StoreReuseStamp>,
    stores: Stores,
}

#[derive(Default)]
pub(crate) struct Pool {
    idle: Mutex<Vec<Idle>>,
    #[cfg(test)]
    opens: std::sync::atomic::AtomicUsize,
}

pub(super) struct Lease {
    pool: Option<Arc<Pool>>,
    idle: Option<Idle>,
    accepted: Cell<bool>,
}

fn stamps(stores: &Stores) -> Result<Vec<StoreReuseStamp>> {
    std::iter::once(&stores.0)
        .chain(stores.1.iter())
        .map(Store::reuse_stamp)
        .collect()
}

impl Lease {
    pub(super) fn open(
        config: &Config,
        route: &Path,
        env: &InvocationEnv,
        verify: VerifyRestores,
    ) -> Result<Self> {
        let pool = INVOCATION_SETTINGS.with(|slot| {
            slot.borrow()
                .as_ref()
                .and_then(|settings| settings.daemon.as_ref())
                .map(|daemon| Arc::clone(&daemon.compiler_stores))
        });
        Self::open_with_pool(config, route, env, verify, pool)
    }

    fn open_with_pool(
        config: &Config,
        route: &Path,
        env: &InvocationEnv,
        verify: VerifyRestores,
        pool: Option<Arc<Pool>>,
    ) -> Result<Self> {
        if pool.is_none() {
            let stores = open_primary_and_fallback_for(config, route, env, verify)?;
            return Ok(Self {
                pool: None,
                idle: Some(Idle {
                    key: Key {
                        directory: PathBuf::new(),
                        config: String::new(),
                    },
                    stamps: Vec::new(),
                    stores,
                }),
                accepted: Cell::new(false),
            });
        }
        crate::link::set_mapped_target(config.volume_store_for_invocation(route, env).is_some());
        let routed = config.routed_for_path_for(route, env);
        let options = StoreOptions::from_env_with_verify_default(|name| env.var(name).ok(), verify);
        let mut key = Key {
            directory: std::fs::canonicalize(&routed.cache_dir)
                .unwrap_or_else(|_| routed.cache_dir.clone()),
            config: format!("{config:?}|{routed:?}|{options:?}"),
        };
        if let Some(pool) = &pool {
            let candidate = {
                let mut idle = pool
                    .idle
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                idle.iter()
                    .position(|entry| entry.key == key)
                    .map(|index| idle.remove(index))
            };
            if let Some(candidate) = candidate
                && stamps(&candidate.stores).is_ok_and(|current| current == candidate.stamps)
            {
                return Ok(Self {
                    pool: Some(Arc::clone(pool)),
                    idle: Some(candidate),
                    accepted: Cell::new(false),
                });
            }
        }
        let stores = open_primary_and_fallback_for(config, route, env, verify)?;
        key.directory = std::fs::canonicalize(&routed.cache_dir)?;
        #[cfg(test)]
        if let Some(pool) = &pool {
            pool.opens
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let complete = super::volume_cache_dirs_match(&routed.cache_dir, &config.cache_dir)
            || !stores.1.is_empty();
        let initial = if complete { stamps(&stores).ok() } else { None };
        Ok(Self {
            pool: initial.as_ref().and(pool),
            idle: Some(Idle {
                key,
                stamps: initial.unwrap_or_default(),
                stores,
            }),
            accepted: Cell::new(false),
        })
    }

    pub(super) fn stores(&self) -> (&Store, &[Store]) {
        let idle = self.idle.as_ref().expect("live store lease");
        (&idle.stores.0, &idle.stores.1)
    }

    pub(super) fn accept_return(&self, accepted: bool) {
        self.accepted.set(accepted);
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if !self.accepted.get() || std::thread::panicking() {
            return;
        }
        let Some(pool) = &self.pool else { return };
        let Some(idle) = self.idle.take() else { return };
        if stamps(&idle.stores).is_ok_and(|current| current == idle.stamps) {
            let mut entries = pool
                .idle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let retired = (entries.len() >= 16).then(|| entries.remove(0));
            entries.push(idle);
            drop(entries);
            drop(retired);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    struct Fixture {
        _dir: tempfile::TempDir,
        config: Config,
        env: InvocationEnv,
        pool: Arc<Pool>,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let config = crate::test_support::test_config(dir.path().join("cache"));
            let env = InvocationEnv::from_parts(
                std::iter::empty::<(&str, &str)>(),
                Some(dir.path().to_owned()),
                None,
                dir.path().to_owned(),
            );
            Self {
                _dir: dir,
                config,
                env,
                pool: Arc::new(Pool::default()),
            }
        }
        fn lease(&self) -> Lease {
            Lease::open_with_pool(
                &self.config,
                &self.config.cache_dir,
                &self.env,
                VerifyRestores::Always,
                Some(Arc::clone(&self.pool)),
            )
            .unwrap()
        }
        fn finish(&self, lease: Lease) {
            lease.accept_return(true);
            drop(lease);
        }
        fn opens(&self) -> usize {
            self.pool.opens.load(Ordering::Relaxed)
        }
    }

    #[test]
    fn reuses_idle_store_and_prepared_reads() {
        let f = Fixture::new();
        for _ in 0..4 {
            let lease = f.lease();
            assert!(!lease.stores().0.contains("absent"));
            f.finish(lease);
        }
        assert_eq!(f.opens(), 1);
    }

    #[test]
    fn replacement_reopens_even_with_identical_schema_and_versions() {
        let f = Fixture::new();
        f.finish(f.lease());
        let old = f.config.cache_dir.with_file_name("retired");
        std::fs::rename(&f.config.cache_dir, &old).unwrap();
        drop(Store::open(&f.config).unwrap());
        f.finish(f.lease());
        assert_eq!(f.opens(), 2);
    }

    #[test]
    fn clear_from_another_process_invalidates_idle_handle() {
        let f = Fixture::new();
        f.finish(f.lease());
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "wrapper::store_handles::tests::clear_worker"])
            .env("KACHE_STORE_CLEAR_TEST_CACHE", &f.config.cache_dir)
            .status();
        assert!(status.unwrap().success());
        f.finish(f.lease());
        assert_eq!(f.opens(), 2);
    }

    #[test]
    fn clear_worker() {
        if let Some(path) = std::env::var_os("KACHE_STORE_CLEAR_TEST_CACHE") {
            Store::open(&crate::test_support::test_config(path.into()))
                .unwrap()
                .clear()
                .unwrap();
        }
    }

    #[test]
    fn schema_and_user_generation_changes_reopen() {
        for sql in [
            "ALTER TABLE entries ADD COLUMN extra_test INTEGER",
            "PRAGMA user_version=13",
            "PRAGMA application_id=41",
        ] {
            let f = Fixture::new();
            let lease = f.lease();
            lease
                .stores()
                .0
                .file_hash_cache()
                .connection()
                .execute_batch(sql)
                .unwrap();
            f.finish(lease);
            f.finish(f.lease());
            assert_eq!(f.opens(), 2, "{sql}");
        }
    }

    #[test]
    fn changed_rule_columns_are_repaired_on_reopen() {
        let f = Fixture::new();
        let lease = f.lease();
        lease
            .stores()
            .0
            .file_hash_cache()
            .connection()
            .execute_batch("ALTER TABLE file_hashes DROP COLUMN rule")
            .unwrap();
        f.finish(lease);
        let next = f.lease();
        assert!(next.stores().0.file_hash_cache().connection().query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('file_hashes') WHERE name='rule')", [], |r| r.get::<_, bool>(0)).unwrap());
        f.finish(next);
        assert_eq!(f.opens(), 2);
    }

    #[test]
    fn sqlite_errors_are_sticky_after_successful_queries_and_rollbacks() {
        for query in ["not valid SQL", "SELECT abs(-9223372036854775808)"] {
            let f = Fixture::new();
            let lease = f.lease();
            let cache = lease.stores().0.file_hash_cache();
            assert!(
                cache
                    .connection()
                    .query_row(query, [], |r| r.get::<_, i64>(0))
                    .is_err()
            );
            assert_eq!(
                cache
                    .connection()
                    .query_row("SELECT 1", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                1
            );
            drop(cache);
            f.finish(lease);
            f.finish(f.lease());
            assert_eq!(f.opens(), 2, "{query}");
        }
    }

    #[test]
    fn configuration_directory_and_restore_policy_never_cross() {
        let mut f = Fixture::new();
        f.finish(f.lease());
        f.config.max_size += 1;
        f.finish(f.lease());
        f.config.cache_dir = f._dir.path().join("other");
        f.finish(f.lease());
        let other_policy = Lease::open_with_pool(
            &f.config,
            &f.config.cache_dir,
            &f.env,
            VerifyRestores::Off,
            Some(Arc::clone(&f.pool)),
        )
        .unwrap();
        f.finish(other_policy);
        assert_eq!(f.opens(), 4);
    }

    #[test]
    fn concurrent_leases_are_exclusive_and_see_external_commits() {
        let f = Fixture::new();
        f.finish(f.lease());
        let barrier = Arc::new(std::sync::Barrier::new(4));
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let barrier = Arc::clone(&barrier);
                let f = &f;
                scope.spawn(move || {
                    let lease = f.lease();
                    barrier.wait();
                    assert!(!lease.stores().0.contains("absent"));
                    f.finish(lease);
                });
            }
        });
        assert_eq!(f.opens(), 4);
        let writer = Store::open(&f.config).unwrap();
        writer
            .file_hash_cache()
            .connection()
            .execute_batch(
                "INSERT INTO entries(cache_key,crate_name,size,committed) VALUES('external','external',1,1)",
            )
            .unwrap();
        let next = f.lease();
        assert!(
            next.stores()
                .0
                .file_hash_cache()
                .connection()
                .query_row(
                    "SELECT committed FROM entries WHERE cache_key='external'",
                    [],
                    |row| row.get::<_, bool>(0)
                )
                .unwrap()
        );
        f.finish(next);
        assert_eq!(f.opens(), 5);
    }

    #[test]
    fn standalone_and_failed_requests_are_not_pooled() {
        let f = Fixture::new();
        drop(f.lease());
        f.finish(f.lease());
        assert_eq!(f.opens(), 2);
        let lease = Lease::open_with_pool(
            &f.config,
            &f.config.cache_dir,
            &f.env,
            VerifyRestores::Always,
            None,
        )
        .unwrap();
        assert!(lease.pool.is_none());
        f.finish(lease);
        assert_eq!(f.opens(), 2);
    }

    #[test]
    fn active_transactions_are_never_returned() {
        let f = Fixture::new();
        let lease = f.lease();
        lease
            .stores()
            .0
            .file_hash_cache()
            .connection()
            .execute_batch("BEGIN IMMEDIATE")
            .unwrap();
        f.finish(lease);
        f.finish(f.lease());
        assert_eq!(f.opens(), 2);
    }

    #[test]
    fn idle_handles_are_bounded_but_active_requests_are_not_serialized() {
        let f = Fixture::new();
        let leases: Vec<_> = (0..17).map(|_| f.lease()).collect();
        for lease in leases {
            f.finish(lease);
        }
        assert_eq!(f.opens(), 17);
        assert_eq!(f.pool.idle.lock().unwrap().len(), 16);
    }

    #[test]
    fn saturated_pool_retains_new_configs_in_return_order() {
        let mut f = Fixture::new();
        let original_size = f.config.max_size;
        let leases: Vec<_> = (0..16).map(|_| f.lease()).collect();
        for lease in leases {
            f.finish(lease);
        }
        assert_eq!(f.opens(), 16);

        f.config.max_size = original_size + 1;
        f.finish(f.lease());
        f.finish(f.lease());
        assert_eq!(f.opens(), 17);

        f.config.max_size = original_size;
        f.finish(f.lease());
        f.config.max_size = original_size + 2;
        f.finish(f.lease());
        f.config.max_size = original_size + 1;
        f.finish(f.lease());
        assert_eq!(f.opens(), 18);
        assert_eq!(f.pool.idle.lock().unwrap().len(), 16);
    }

    #[test]
    fn moved_corrupt_index_uses_existing_recovery() {
        let f = Fixture::new();
        f.finish(f.lease());
        std::fs::rename(&f.config.cache_dir, f._dir.path().join("retired")).unwrap();
        std::fs::create_dir(&f.config.cache_dir).unwrap();
        std::fs::write(
            f.config.cache_dir.join("index.db"),
            b"invalid sqlite database",
        )
        .unwrap();
        let next = f.lease();
        assert_eq!(next.stores().0.blob_stats().unwrap().total_blobs, 0);
        f.finish(next);
        assert_eq!(f.opens(), 2);
    }

    #[test]
    fn unavailable_fallback_is_retried_next_request() {
        let mut f = Fixture::new();
        f.config.volume_stores.push(crate::config::VolumeStore {
            volume: format!("{}/", f._dir.path().display()),
            store: f._dir.path().join("shard"),
            max_size: Some(1024 * 1024),
        });
        let route = f._dir.path().join("unit.o");
        let broken_index = f.config.cache_dir.join("index.db");
        std::fs::create_dir_all(&broken_index).unwrap();
        let open = || {
            Lease::open_with_pool(
                &f.config,
                &route,
                &f.env,
                VerifyRestores::Always,
                Some(Arc::clone(&f.pool)),
            )
            .unwrap()
        };
        let first = open();
        assert!(first.stores().1.is_empty());
        f.finish(first);
        std::fs::remove_dir(&broken_index).unwrap();
        let next = open();
        assert_eq!(next.stores().1.len(), 1);
        f.finish(next);
        assert_eq!(f.opens(), 2);
    }
}
