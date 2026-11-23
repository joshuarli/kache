"""Verify retained acceptance evidence; optionally write the derived summary."""
import collections
import hashlib
import json
import math
from pathlib import Path
import statistics
import sys

HERE = Path(__file__).resolve().parent
REVISION = 'ee2402bfb20c2d018c4036bec5e53a1084f79a9a'
T975 = [12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262]
analysis = {'source_revision': REVISION, 'summary': [], 'wall_losses': []}
ratio_count = replay_count = observation_count = 0
for tc in ['gcc', 'clang']:
    path = HERE / tc / 'report.json'
    report = json.loads(path.read_text())
    old = json.loads((HERE.parent / '2026-11-21-l1-c4d2b947' / tc / 'report.json').read_text())
    assert report['mode'] == 'acceptance'
    assert report['provenance']['source_revision'] == REVISION
    assert report['provenance']['source_dirty'] is False
    assert report['seed'] == old['seed'] == 20261117
    assert report['jobs'] == old['jobs'] == 4
    assert report['compiler'] == old['compiler']
    for workload in report['workloads']:
        name = workload['workload']
        previous = next(w for w in old['workloads'] if w['workload'] == name)
        assert workload['input_digest'] == previous['input_digest']
        assert not workload['validation_failures'], workload['validation_failures']
        assert len(workload['validation']) == 3
        paired = collections.defaultdict(dict)
        activities = []
        for replay in workload['timed']:
            replay_count += 1
            wall = math.fsum(o['activity']['wall_ms'] for o in replay['observations'])
            cpu = math.fsum(o['activity']['unrelated_cpu_ms'] for o in replay['observations'])
            activities.append({'pair': replay['pair'], 'arm': replay['arm'],
                               'admitted': replay['admitted'], 'unrelated_cores': cpu / wall})
            for observation in replay['observations']:
                assert observation['codegen'] is None, 'Observer must be disabled for timing'
                observation_count += 1
            if replay['admitted']:
                paired[replay['pair']][replay['arm']] = {
                    o['scenario']: o for o in replay['observations']}
        complete = {p: arms for p, arms in paired.items() if set(arms) == {'raw', 'ccache', 'kache'}}
        assert len(complete) == workload['acceptance']['complete_pairs']
        for arms in complete.values():
            for scenario, raw in arms['raw'].items():
                for arm in ['kache', 'ccache']:
                    assert arms[arm][scenario]['objects'] == raw['objects']
                    assert arms[arm][scenario]['object_count'] == raw['object_count']
        targets = []
        for ratio in workload['ratios']:
            scenario, other = ratio['scenario'], ratio['against']
            logs = [math.log(arms['kache'][scenario]['wall_ms'] / arms[other][scenario]['wall_ms'])
                    for arms in complete.values()]
            summary = ratio['summary']
            assert len(logs) == summary['pairs']
            mean = statistics.mean(logs)
            assert math.isclose(math.exp(mean), summary['geomean'], rel_tol=1e-10)
            if 2 <= len(logs) <= 10:
                half = T975[len(logs)-2] * math.sqrt(statistics.variance(logs)/len(logs))
                ci = [math.exp(mean-half), math.exp(mean+half)]
                assert all(math.isclose(a,b,rel_tol=1e-10) for a,b in zip(ci,summary['ci95']))
            if ratio['target'] is not None:
                low, high = summary['ci95']
                verdict = 'PASS' if high <= ratio['target'] else 'FAIL' if low > ratio['target'] else 'INCONCLUSIVE'
                assert ratio['verdict'] == verdict
                targets.append(ratio)
            ratio_count += 1
            if other == 'ccache':
                excess = [arms['kache'][scenario]['wall_ms']-arms[other][scenario]['wall_ms']
                          for arms in complete.values()]
                analysis['wall_losses'].append({'toolchain':tc,'workload':name,'scenario':scenario,
                    'mean_paired_excess_ms':statistics.mean(excess),'pairs':len(excess),
                    'eligible':workload['acceptance']['eligible']})
        row = {'toolchain':tc,'workload':name,'acceptance':workload['acceptance'],
            'raw_report_sha256':hashlib.sha256(path.read_bytes()).hexdigest(),
            'all_targets':targets,'replay_activity':activities,
            'max_unrelated_cores':max(a['unrelated_cores'] for a in activities),
            'admitted_replays':sum(a['admitted'] for a in activities),
            'eligible_replays':sum(a['unrelated_cores']<=0.5 for a in activities),
            'individual_build_windows_eligible':sum(o['activity']['eligible'] for r in workload['timed'] for o in r['observations']),
            'individual_build_windows':sum(len(r['observations']) for r in workload['timed']),
            'comparison_114':{r['scenario']:r for r in previous['ratios'] if r['against']=='ccache'}}
        analysis['summary'].append(row)
        brief = {(r['scenario'],r['against']):r for r in workload['ratios']}
        print(tc,name,workload['acceptance']['verdict'].upper(),len(complete),'pairs',
              'cold',brief['cold','ccache']['summary'],'warm',brief['warm','ccache']['summary'])
analysis['wall_losses'].sort(key=lambda r:r['mean_paired_excess_ms'],reverse=True)
if '--write' in sys.argv:
    (HERE/'analysis.json').write_text(json.dumps(analysis,indent=2)+'\n')
else:
    assert json.loads((HERE/'analysis.json').read_text()) == analysis
print(f'PASS: {ratio_count} ratio estimates/95% log-t intervals; all locked target verdicts reproduced')
print(f'PASS: {replay_count} retained timed replays, {observation_count} observer-free observations; complete-pair objects equal raw')
print('PASS: clean exact revision, jobs/seed/compiler/input-digest continuity with #114; no validation failures')
