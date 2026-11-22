"""Recompute checkpoint timing intervals from the committed observations."""

import json
from pathlib import Path
import random
import statistics


ROOT = Path(__file__).resolve().parent
DRAWS = 50_000


def load(name):
    return json.loads((ROOT / name).read_text())


def interval(values):
    values = sorted(values)

    def quantile(p):
        at = (len(values) - 1) * p
        lo = int(at)
        hi = min(lo + 1, len(values) - 1)
        return values[lo] + (values[hi] - values[lo]) * (at - lo)

    return [quantile(0.025), quantile(0.975)]


def estimate(aa, bb, seed, paired_delta=False):
    assert len(aa) == len(bb) and len(aa) >= 5
    rng = random.Random(seed)
    draws = []
    for _ in range(DRAWS):
        indices = [rng.randrange(len(aa)) for _ in aa]
        a = statistics.median(aa[i] for i in indices)
        b = statistics.median(bb[i] for i in indices)
        delta = statistics.median(bb[i] - aa[i] for i in indices) if paired_delta else b - a
        draws.append((a, b, b / a, delta))
    a, b = statistics.median(aa), statistics.median(bb)
    delta = statistics.median(y - x for x, y in zip(aa, bb)) if paired_delta else b - a
    return dict(pairs=len(aa), A_ms=a, B_ms=b, ratio=b / a, delta_ms=delta,
                A_ci95_ms=interval([d[0] for d in draws]),
                B_ci95_ms=interval([d[1] for d in draws]),
                ratio_ci95=interval([d[2] for d in draws]),
                delta_ci95_ms=interval([d[3] for d in draws]))


def equal(actual, expected):
    if isinstance(actual, list):
        assert len(actual) == len(expected)
        for a, b in zip(actual, expected):
            equal(a, b)
    else:
        assert abs(actual - expected) < 1e-8, (actual, expected)


def main():
    result = dict(endpoint=[], bisect=[], losses={})
    endpoints = load('endpoint-samples.json')
    assert len(endpoints) == 80
    assert all(r['hits_per_warm'] == 12 and r['objects_match_raw'] for r in endpoints)
    assert all(r[k] == 0 for r in endpoints for k in
               ['compiler_runs_per_warm', 'preprocessor_runs_per_warm', 'probe_runs_per_warm'])
    for ref in load('endpoint-analysis.json')['results']:
        rows = [r for r in endpoints if r['tc'] == ref['compiler'] and r['mode'] == ref['mode']]
        arms = [sorted([r for r in rows if r['version'] == arm], key=lambda r: r['pair'])
                for arm in ['A', 'B']]
        assert all([r['pair'] for r in arm] == list(range(10)) for arm in arms)
        for row in rows:
            assert row['start_exit'] == (0 if row['mode'] == 'native' else 1)
            assert (row['daemon_pid'] is not None) == (row['mode'] == 'native')
        stats = estimate(*[[r[ref['phase']] for r in arm] for arm in arms], seed=20261117)
        for key, expected in [('A_ms', ref['A']['median_ms']), ('B_ms', ref['B']['median_ms']),
                              ('A_ci95_ms', ref['A']['ci95_ms']), ('B_ci95_ms', ref['B']['ci95_ms']),
                              ('ratio', ref['ratio_B_over_A']), ('ratio_ci95', ref['ratio_ci95']),
                              ('delta_ms', ref['delta_ms']), ('delta_ci95_ms', ref['delta_ci95_ms'])]:
            equal(stats[key], expected)
        result['endpoint'].append(dict(compiler=ref['compiler'], mode=ref['mode'], phase=ref['phase'], **stats))

    bisect = load('bisect-samples.json')
    assert len(bisect) == 60
    for row in bisect:
        assert row['hits'] == 12 and row['objects_match_raw']
        assert row['compiler_runs'] == row['preprocessor_runs'] == row['probe_runs'] == 0
        assert all(len(pids) == (1 if row['mode'] == 'daemon' else 12) for pids in row['trace_pids'])
    for ref in load('bisect-analysis.json')['summaries']:
        rows = [r for r in bisect if r['label'] == ref['label']]
        arms = [sorted([r for r in rows if r['arm'] == arm], key=lambda r: r['pair']) for arm in [0, 1]]
        assert all([r['pair'] for r in arm] == list(range(ref['pairs'])) for arm in arms)
        stats = estimate(*[[r['warm_ms'] for r in arm] for arm in arms], seed=20261117)
        for key in stats:
            equal(stats[key], ref[key])
        result['bisect'].append(dict(label=ref['label'], A=ref['A'], B=ref['B'], **stats))
    current = result['bisect'][-1]
    result['current_standalone_over_daemon'] = estimate(
        [r['warm_ms'] for r in sorted(bisect, key=lambda r: r['pair']) if r['label'] == 'current3' and r['arm'] == 1],
        [r['warm_ms'] for r in sorted(bisect, key=lambda r: r['pair']) if r['label'] == 'current3' and r['arm'] == 0],
        seed=20261117)
    equal(result['current_standalone_over_daemon']['ratio'], 1 / current['ratio'])

    losses = load('loss-samples.json')
    assert all(r['objects_match_raw'] and not r['profile'] for r in losses)
    for case, scenario, units in [('tiny', 'cold', 48), ('headers', 'edit-header-settled', 12)]:
        rows = [r for r in losses if r['case'] == case and r['scenario'] == scenario]
        arms = [sorted([r for r in rows if r['arm'] == arm], key=lambda r: r['pair']) for arm in ['ccache', 'kache']]
        assert all([r['pair'] for r in arm] == list(range(10)) for arm in arms)
        assert all(r['compiler_runs'] == r['preprocessor_runs'] == units for r in arms[1])
        stats = estimate(*[[r['ms'] for r in arm] for arm in arms], seed=20261122, paired_delta=True)
        ref = load('loss-analysis.json')[case]['timing']
        for key, expected in [('A_ms', ref['ccache_ms']), ('B_ms', ref['kache_ms']),
                              ('A_ci95_ms', ref['ccache_ci']), ('B_ci95_ms', ref['kache_ci']),
                              ('delta_ms', ref['paired_median_excess_ms']), ('delta_ci95_ms', ref['excess_ci'])]:
            equal(stats[key], expected)
        stats['mean_excess_ms'] = statistics.mean(b['ms'] - a['ms'] for a, b in zip(*arms))
        equal(stats['mean_excess_ms'], ref['mean_excess_ms'])
        result['losses'][case] = stats
        for profile in load('loss-analysis.json')[case]['profiles'].values():
            for values in profile.values():
                assert abs(sum(values['critical_chain_budget_ms'].values()) - values['wall_ms']) < 0.05
                assert values['driver_processes']['compile']['count'] == units
                assert values['max_compile_concurrency'] == (1 if case == 'tiny' else 4)

    for record in load('loss-cpu.json').values():
        for category in record['categories'].values():
            equal(category['percent'], category['count'] * 100 / record['samples'])
    (ROOT / 'analysis-summary.json').write_text(json.dumps(result, indent=2) + '\n')
    print('PASS: 40 endpoint pairs, 30 bisect/mode pairs, 20 loss pairs; retained counters and object checks')
    print('PASS: all timing medians and bootstrap intervals reproduce the original analyses')
    print('PASS: profile completion-chain budgets close; CPU category percentages reproduce retained counts')
    print('CHECKPOINT_ANALYSIS_EXIT=0')


if __name__ == '__main__':
    main()
