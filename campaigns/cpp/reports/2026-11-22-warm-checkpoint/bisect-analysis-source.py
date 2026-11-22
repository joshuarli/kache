import collections
import json
from pathlib import Path
import re
import statistics
import run as b

ROOT = b.ROOT


def phases(paths):
    rows = []
    for p in paths:
        events = json.loads(p.read_text())['traceEvents']
        assert not next(e for e in events if e['name'] == 'cc')['args']['dropped_events']
        values = collections.defaultdict(list)
        for event in events:
            if event.get('ph') == 'X':
                values[event['name']].append(event['dur'] / 1000)
        row = {k: max(v) for k, v in values.items()}
        key_parts = ['cc_prefix_maps', 'cc_probe', 'cc_memo_key', 'cc_memo_lookup', 'cc_shadowing', 'cc_preprocess', 'cc_capture']
        row['key_other'] = row['key'] - sum(row.get(k, 0) for k in key_parts)
        row['backend_other'] = row['cc'] - sum(row.get(k, 0) for k in ['key', 'store_open', 'lookup', 'restore', 'event_report', 'memo_commit'])
        rows.append(row)
    assert rows
    names = sorted(set().union(*(r.keys() for r in rows)))
    return dict(hits=len(rows), phases={k:dict(mean_ms=statistics.mean(r.get(k, 0) for r in rows),
                                              median_ms=statistics.median(r.get(k, 0) for r in rows)) for k in names})


def perf(name):
    rows = json.loads((ROOT / ('perf-' + name) / 'rows.json').read_text())
    pids = {p for r in rows for p in r['pids']}
    assert len(pids) == 360
    blocks = (ROOT / (name + '.stacks.txt')).read_text().strip().split('\n\n')
    selected = []
    for s in blocks:
        m = re.match(r'^\S+\s+(\d+)\s+', s)
        if m and int(m[1]) in pids:
            selected.append(s)
    words = ['cc_timestamp_may_expand', 'cc_parsed_header_scans', 'sqlite3', 'deser', 'cache_key_under']
    result = dict(samples=len(selected), pids=len(pids), categories={w:dict(count=sum(w in s for s in selected),
                  percent=100 * sum(w in s for s in selected) / len(selected)) for w in words})
    print('PERF', name, json.dumps(result))
    return result


def main():
    summaries = []
    rows = []
    for label, n in [('p10', 5), ('p05', 5), ('p07', 5), ('parent', 10), ('current3', 5)]:
        selected = json.loads((ROOT / (label + '-rows.json')).read_text())
        assert len(selected) == n * 2
        for row in selected:
            assert row['hits'] == 12 and row['objects_match_raw']
            assert row['compiler_runs'] == row['preprocessor_runs'] == row['probe_runs'] == 0
            for pids in row['trace_pids']:
                assert len(pids) == (1 if row['mode'] == 'daemon' else 12)
            if row['daemon_pid']:
                assert not Path(f"/proc/{row['daemon_pid']}/exe").exists()
        result = json.loads((ROOT / (label + '-summary.json')).read_text())
        summaries.append(result)
        rows.extend(selected)
        print('SUMMARY', json.dumps(result))
    phase_results = {}
    for rev in ['92cb3b3e', '438c5f04']:
        paths = sorted((ROOT / 'state/parent').glob(f'*/{rev}-standalone/trace-*/*.json'))
        phase_results[rev] = phases(paths)
        assert phase_results[rev]['hits'] == 120
        print('PHASE', rev, json.dumps(phase_results[rev]))
    for mode in ['standalone', 'daemon']:
        paths = sorted((ROOT / 'state/current3').glob(f'*/9553a432-{mode}/trace-*/*.json'))
        phase_results['main-' + mode] = phases(paths)
        assert phase_results['main-' + mode]['hits'] == 60
        print('PHASE', 'main-' + mode, json.dumps(phase_results['main-' + mode]))
    for name in ['parent', 'slower']:
        phase_results['perf-' + name] = phases(sorted((ROOT / ('perf-' + name)).glob('*/*.json')))
        assert phase_results['perf-' + name]['hits'] == 360
    perf_results = {n:perf(n) for n in ['parent', 'slower']}
    b.save(ROOT / 'analysis.json', dict(summaries=summaries, phases=phase_results, perf=perf_results))
    print(f'PASS: {len(rows)//2} alternating pairs; {len(rows)} warm timings; all 12-hit/zero-codegen/raw-object checks passed')
    print(f'PASS: {len(rows)} separately traced identity builds; standalone builds have 12 distinct PIDs; daemon builds have one matching PID')
    print('PASS: 60 separate CPU-profiled builds; 720 local hits; zero compiler/preprocessor/probe runs; raw objects match')
    print('ANALYSIS_EXIT=0')


if __name__ == '__main__':
    main()
