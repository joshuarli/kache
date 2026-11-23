import json
import os
from pathlib import Path
import subprocess
import sys

E=Path(__file__).resolve().parent
ROOT=Path('/home/ubuntu/repos/kache/.work/wt/l1-acceptance-rerun')
env=dict(os.environ)
for key in ['TMPDIR','KACHE_CACHE_DIR']:
    env.pop(key,None)
assert subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip()=='ee2402bfb20c2d018c4036bec5e53a1084f79a9a'
assert not subprocess.check_output(['git','status','--porcelain'],cwd=ROOT,text=True).strip()
watcher=None
(E/'stop-watcher').unlink(missing_ok=True)
try:
    for tc in sys.argv[1:] or ['gcc','clang']:
        assert not (ROOT/f'.work/tmp/cc-bench/{tc}/report.json').exists(), 'Preserve the existing report'
        with (E/f'preflight-{tc}.txt').open('w') as log:
            result=subprocess.run([str(E/'target/release/preflight-only'),tc],env=env,cwd=ROOT,stdout=log,stderr=subprocess.STDOUT)
        print((E/f'preflight-{tc}.txt').read_text(),flush=True)
        assert result.returncode==0,'Quiet-host preflight failed; no benchmark launched'
        if watcher is None:
            watcher=subprocess.Popen(['python',str(E/'daemon_watch.py')],stdout=(E/'daemon-watch.log').open('w'),stderr=subprocess.STDOUT)
        command=['make','cc-bench',f'TOOLCHAINS={tc}',
            'CC_ORACLE_CCACHE_DIR=/home/ubuntu/repos/kache/.work/wt/cc-bench-l1-rerun/.work/tmp/ccache',
            'BENCH_ARGS=--acceptance --jobs 4 --workload corpus-c-tiny --workload corpus-cxx-headers --workload zlib']
        with (E/f'{tc}-acceptance.txt').open('w') as log:
            log.write('COMMAND: '+repr(command)+'\n'); log.flush()
            result=subprocess.run(command,cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT)
            log.write(f'\nACCEPTANCE_EXIT={result.returncode}\n')
        print(f'{tc}: ACCEPTANCE_EXIT={result.returncode}',flush=True)
        report=json.loads((ROOT/f'.work/tmp/cc-bench/{tc}/report.json').read_text())
        assert report['provenance']['source_revision']=='ee2402bfb20c2d018c4036bec5e53a1084f79a9a'
        assert report['provenance']['source_dirty'] is False
        for workload in report['workloads']:
            print(tc,workload['workload'],json.dumps(workload['acceptance']),flush=True)
            assert not workload['validation_failures'],workload['validation_failures']
        assert watcher.poll() is None,'Daemon observer failed'
finally:
    (E/'stop-watcher').touch()
    if watcher is not None:
        assert watcher.wait(timeout=10)==0
