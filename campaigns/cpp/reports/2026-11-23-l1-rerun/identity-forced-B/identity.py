"""Untimed native-daemon warm-path verification after all acceptance timing."""
import hashlib
import json
from pathlib import Path
import shlex
import subprocess
import tempfile

E=Path(__file__).resolve().parent
ROOT=Path('/home/ubuntu/repos/kache/.work/wt/l1-acceptance-rerun')
KACHE=ROOT/'target/release/kache'
ENGINE=ROOT/'target/release/kache-engine'
for tc in ['gcc','clang']:
    report=json.loads((ROOT/f'.work/tmp/cc-bench/{tc}/report.json').read_text())
    workload=next(w for w in report['workloads'] if w['workload']=='corpus-cxx-headers')
    last=[r for r in workload['timed'] if r['arm']=='kache'][-1]
    command=shlex.split(last['observations'][-1]['command'])
    assert command[0]=='cd' and command[2]=='&&'
    cwd=Path(command[1]); argv=[command[3],'-B']+command[4:]
    with tempfile.TemporaryDirectory(prefix='kache-l1-',dir='/tmp') as runtime:
        env=dict(workload['environments']['kache'])
        env['KACHE_RUNTIME_DIR']=runtime
        env['KACHE_BASE_DIR']=str(cwd)
        def cli(args):
            return subprocess.run([str(KACHE)]+args,cwd=cwd,env=env,capture_output=True,text=True,check=True)
        cli(['daemon','start'])
        try:
            status=json.loads(cli(['--json','daemon','status']).stdout)
            assert status['daemon_running'] and status['daemon_epoch']==status['epoch']
            state=json.loads((Path(runtime)/'daemon.state.json').read_text())
            assert Path(f'/proc/{state["pid"]}/exe').resolve()==ENGINE
            trace=E/f'{tc}-warm-identity.strace'
            result=subprocess.run(['/usr/bin/strace','-f','-e','execve','-o',str(trace)]+argv,
                cwd=cwd,env=env,capture_output=True,text=True,check=True)
            (E/f'{tc}-warm-identity-build.txt').write_text(result.stdout+result.stderr)
            events=[json.loads(line) for line in (Path(runtime)/'events.jsonl').read_text().splitlines()]
            (E/f'{tc}-warm-identity-events.jsonl').write_text((Path(runtime)/'events.jsonl').read_text())
            assert len(events)==12 and all(e['result']=='local_hit' for e in events)
            assert all(e['compiler_runs']==0 and e['preprocessor_runs']==0 for e in events)
            executions=trace.read_text()
            client_execs=executions.count('execve("'+str(KACHE)+'",')
            engine_execs=executions.count('execve("'+str(ENGINE)+'",')
            assert client_execs==12 and engine_execs==0,(client_execs,engine_execs)
            evidence=dict(toolchain=tc,status=status,pid=state['pid'],runtime=runtime,
                engine_sha256=hashlib.sha256(ENGINE.read_bytes()).hexdigest(),
                client_execs=client_execs,engine_execs=engine_execs,hits=len(events),
                compiler_runs=0,preprocessor_runs=0,command=argv,cwd=str(cwd),
                scope='untimed post-acceptance identity check, excluded from every performance sample')
            (E/f'{tc}-warm-identity.json').write_text(json.dumps(evidence,indent=2)+'\n')
            print('PASS:',tc,'12 warm requests, 12 hits, zero compiler/preprocessor/standalone-engine executions; native daemon',state['pid'])
        finally:
            cli(['daemon','stop'])
