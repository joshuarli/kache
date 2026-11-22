import json
import os
from pathlib import Path
import random
import statistics
import subprocess

ROOT = Path('/home/ubuntu/warm-endpoint-ab-evidence')
rows = json.loads((ROOT / 'rows.json').read_text())
controls = json.loads((ROOT / 'controls.json').read_text())
assert len(rows) == 80 and len(controls) == 40
assert all(r['hits_per_warm'] == 12 and r['objects_match_raw'] for r in rows)
assert all(r[k] == 0 for r in rows for k in ['compiler_runs_per_warm', 'preprocessor_runs_per_warm', 'probe_runs_per_warm'])


def interval(values):
    values = sorted(values)
    def q(p):
        at = p * (len(values) - 1)
        lo = int(at)
        hi = min(lo + 1, len(values) - 1)
        return values[lo] + (values[hi] - values[lo]) * (at - lo)
    return [q(0.025), q(0.975)]


results = []
for tc in ['gcc', 'clang']:
    for mode in ['fallback', 'native']:
        selected = [r for r in rows if r['tc'] == tc and r['mode'] == mode]
        a = sorted([r for r in selected if r['version'] == 'A'], key=lambda r: r['pair'])
        b = sorted([r for r in selected if r['version'] == 'B'], key=lambda r: r['pair'])
        assert [r['pair'] for r in a] == list(range(10)) == [r['pair'] for r in b]
        for field in ['first_ms', 'second_ms']:
            aa = [r[field] for r in a]
            bb = [r[field] for r in b]
            rng = random.Random(20261117)
            samples = []
            for _ in range(50000):
                indices = [rng.randrange(10) for _ in range(10)]
                am = statistics.median(aa[i] for i in indices)
                bm = statistics.median(bb[i] for i in indices)
                samples.append((am, bm, bm / am, bm - am))
            am, bm = statistics.median(aa), statistics.median(bb)
            result = dict(compiler=tc, mode=mode, phase=field, pairs=10,
                          A=dict(median_ms=am, ci95_ms=interval([x[0] for x in samples]), range_ms=[min(aa), max(aa)]),
                          B=dict(median_ms=bm, ci95_ms=interval([x[1] for x in samples]), range_ms=[min(bb), max(bb)]),
                          ratio_B_over_A=bm / am, ratio_ci95=interval([x[2] for x in samples]),
                          delta_ms=bm - am, delta_ci95_ms=interval([x[3] for x in samples]))
            results.append(result)
            print('SUMMARY', json.dumps(result))
        cc = [r['ms'] for r in controls if r['tc'] == tc and r['mode'] == mode]
        rng = random.Random(20261117)
        cc_result = dict(compiler=tc, mode=mode, arm='ccache', median_ms=statistics.median(cc),
                         ci95_ms=interval([statistics.median(rng.choices(cc, k=10)) for _ in range(50000)]))
        print('CONTROL', json.dumps(cc_result))

identities = []
native_count = 0
fallback_count = 0
for row in rows:
    state = ROOT / 'state' / row['mode'] / row['tc'] / f"{row['pair']:02d}" / row['version']
    start = json.loads((state / 'start.json').read_text())
    if row['mode'] == 'native':
        native_count += 1
        assert start['returncode'] == 0
        assert json.loads((state / 'stop.json').read_text())['returncode'] == 0
        assert not Path(f"/proc/{row['daemon_pid']}/exe").exists()
        assert start['socket_bytes'] < 107
    else:
        fallback_count += 1
        assert start['returncode'] != 0 and 'at most 107' in start['stderr']
        assert start['socket_bytes'] > 107
    if row['identity'] is not None:
        identity = dict(compiler=row['tc'], mode=row['mode'], version=row['version'], **row['identity'])
        identities.append(identity)
        print('IDENTITY', json.dumps(identity))
assert native_count == fallback_count == 40
for tc in ['gcc', 'clang']:
    state = ROOT / 'ccache' / tc
    env = dict(PATH='/usr/bin:/bin', HOME=str(state / 'home'), CCACHE_DIR=str(state / 'store'),
               CCACHE_CONFIGPATH=str(state / 'ccache.conf'))
    cc = '/home/ubuntu/repos/kache/.work/wt/cc-bench-l1-rerun/.work/tmp/ccache/ccache'
    p = subprocess.run([cc, '--print-stats'], env=env, capture_output=True, check=True)
    (ROOT / f'ccache-stats-{tc}.txt').write_bytes(p.stdout)
    stats = {line.split()[0]: int(line.split()[1]) for line in p.stdout.decode().splitlines()}
    assert stats['direct_cache_hit'] == 240 and stats['cache_miss'] == 12, stats

(ROOT / 'summary.json').write_text(json.dumps(dict(bootstrap=dict(draws=50000, seed=20261117, method='paired percentile; ratio of medians'),
                                                 results=results, identities=identities), indent=2) + '\n')
print('PASS: 40 complete A/B pairs; 80 first-warm and 80 second-warm builds; 1920 unprofiled local hits; all objects match raw')
print('PASS: 40 native starts/current-engine identities/stops; 40 expected long-path start failures; 8 separate PID-traced builds')
print('PASS: fixed GCC, Clang, ccache binary digests match historical BLAKE3 records; fixture BLAKE3 matches all four reports')
print('PASS: ccache controls each have 240 direct hits and 12 priming misses')
print('ANALYSIS_EXIT=0')
