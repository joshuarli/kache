import hashlib
import json
from pathlib import Path
import time
import sys

ROOT=Path(__file__).resolve().parent
ENGINE=Path('/home/ubuntu/repos/kache/.work/wt/l1-acceptance-rerun/target/release/kache-engine')
seen=set()
with (ROOT/f'daemon-observations-{sys.argv[1] if len(sys.argv)>1 else "clang"}.jsonl').open('w') as output:
    while not (ROOT/'stop-watcher').exists():
        for runtime in Path('/tmp').glob('kache-cc-*'):
            try:
                state=json.loads((runtime/'daemon.state.json').read_text())
                pid=state['pid']
                if pid in seen: continue
                actual=Path(f'/proc/{pid}/exe').resolve()
                if actual!=ENGINE: continue
                mode=runtime.stat().st_mode & 0o777
                assert mode==0o700,(runtime,mode)
                sockets=[str(p) for p in runtime.iterdir() if p.is_socket()]
                assert all(len(p.encode())<104 for p in sockets),sockets
                actual_digest=hashlib.sha256(Path(f'/proc/{pid}/exe').read_bytes()).hexdigest()
                digest=hashlib.sha256(ENGINE.read_bytes()).hexdigest()
                assert actual_digest==digest
                output.write(json.dumps(dict(pid=pid,runtime=str(runtime),mode=oct(mode),
                    sockets=sockets,exe=str(actual),sha256=actual_digest,time=time.time()))+'\n')
                output.flush()
                seen.add(pid)
            except (FileNotFoundError,ProcessLookupError,json.JSONDecodeError,KeyError): pass
        time.sleep(1)
print('PASS: sampled',len(seen),'native benchmark daemon PIDs; executable/digest/private runtime/short sockets verified')
