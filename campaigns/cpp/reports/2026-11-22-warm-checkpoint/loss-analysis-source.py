import collections
import json
from pathlib import Path
import random
import statistics

R=Path('/home/ubuntu/l1-loss-phase-evidence')
def save(p,v):p.write_text(json.dumps(v,indent=2)+'\n')
def percentile(xs,q):
 xs=sorted(xs);i=(len(xs)-1)*q;j=int(i)
 return xs[j]+(xs[min(j+1,len(xs)-1)]-xs[j])*(i-j)
def ci(xs):return [percentile(xs,.025),percentile(xs,.975)]
def stats(case,scenario):
 rows=[x for x in json.loads((R/f'paired-{case}-rows.json').read_text())if x['scenario']==scenario]
 arms={arm:{x['pair']:x['ms']for x in rows if x['arm']==arm}for arm in ['kache','ccache']}
 assert arms['kache'].keys()==arms['ccache'].keys()
 pairs=sorted(arms['kache']);a=[arms['kache'][p]for p in pairs];b=[arms['ccache'][p]for p in pairs];d=[x-y for x,y in zip(a,b)]
 rng=random.Random(20261122);boots=[];ba=[];bb=[]
 for _ in range(50000):
  ix=[rng.randrange(len(pairs))for _ in pairs]
  boots.append(statistics.median(d[i]for i in ix));ba.append(statistics.median(a[i]for i in ix));bb.append(statistics.median(b[i]for i in ix))
 return dict(pairs=len(pairs),kache_ms=statistics.median(a),kache_ci=ci(ba),ccache_ms=statistics.median(b),ccache_ci=ci(bb),paired_median_excess_ms=statistics.median(d),excess_ci=ci(boots),paired_excesses_ms=d,mean_excess_ms=statistics.mean(d))

def procs(state,row):
 out=[]
 for line in (state/'processes.tsv').read_text().splitlines():
  a,b,pid,kind,user,system,args=line.split('\t')
  a=int(a)/1e6;b=int(b)/1e6
  if not row['start_ns']/1e6<=a<=row['end_ns']/1e6:continue
  argv=args.rstrip('\x1f').split('\x1f')
  source=next((Path(arg).name for arg in argv if arg.endswith(('.c','.cpp','.i','.ii'))),'none')
  out.append(dict(a=a,b=b,kind=kind,source=source,pid=int(pid),cpu_ms=(int(user)+int(system))/1000,argv=argv))
 return out

def overlap(intervals):
 points=sorted([(p['a'],1)for p in intervals]+[(p['b'],-1)for p in intervals]);n=0;maximum=0
 for _,d in points:n+=d;maximum=max(maximum,n)
 return maximum

def profile(label,case,scenario,arm):
 state=R/f'state/{label}/{case}/00/{arm}'
 row=next(x for x in json.loads((state/'rows.json').read_text())if x['scenario']==scenario)
 processes=procs(state,row)
 clients=sorted([p for p in processes if p['kind']=='client'],key=lambda p:p['a'])
 compilers=[p for p in processes if p['kind']=='compile']
 assert len(clients)==len(compilers)==(48 if case=='tiny'else 12)
 compiler_by_source={p['source']:p for p in compilers};assert len(compiler_by_source)==len(compilers)
 backends=[]
 if arm=='kache':
  for f in (state/f'traces-{scenario}').glob('*.json'):
   events=json.loads(f.read_text())['traceEvents'];root=events[0]
   assert root['name']=='cc' and root['pid']==row['daemon_pid']
   phases=[dict(a=e['ts']/1000,b=(e['ts']+e['dur'])/1000,name=e['name'])for e in events if e['ph']=='X']
   cc=next(p for p in phases if p['name']=='cc_compile')
   backends.append(dict(a=root['ts']/1000,b=(root['ts']+root['dur'])/1000,phases=phases,cc=cc))
  edges=sorted(((abs(t['cc']['a']-p['a'])+abs(t['cc']['b']-p['b']),i,j)for i,t in enumerate(backends)for j,p in enumerate(compilers)))
  used_t=set();used_p=set()
  for cost,i,j in edges:
   if i in used_t or j in used_p:continue
   assert cost<15,(case,cost)
   backends[i]['source']=compilers[j]['source'];backends[i]['compiler_alignment_error_ms']=cost
   used_t.add(i);used_p.add(j)
  assert len(used_t)==len(backends)
 by_source={p['source']:p for p in backends}
 for client in clients:
  t=by_source.get(client['source']);client['backend']=t
  if t:assert client['a']<=t['a']<=t['b']<=client['b']
 finished=sorted(clients,key=lambda p:p['b']);jobs=1 if case=='tiny'else 4
 for i,c in enumerate(clients):
  c['pred']=finished[i-jobs]if i>=jobs else None
  assert not c['pred']or c['pred']['b']<=c['a'],(case,'non-causal scheduling edge')
 chain=[];c=finished[-1]
 while c:chain.append(c);c=c['pred']
 chain.reverse();budget=collections.Counter()
 priority=['compiler_process','preprocessor_process','probe_process','permit_wait','flight_join','key_analysis','store','restore','lookup','store_open','memo_commit','event_report','compiler_output_work','backend_other','frontend_or_IPC','ccache_other']
 mapping={'key':'key_analysis','cc_compile':'compiler_output_work'}
 for client in chain:
  segs=[]
  for p in processes:
   if p['kind']=='client' or p['a']<client['a'] or p['b']>client['b']:continue
   if p['kind']in ['compile','preprocess'] and p['source']not in [client['source'],'epoch.c']:continue
   kind={'compile':'compiler_process','preprocess':'preprocessor_process','probe':'probe_process'}.get(p['kind'])
   if kind:segs.append((p['a'],p['b'],kind))
  t=client['backend']
  if t:
   segs.append((t['a'],t['b'],'backend_other'))
   for p in t['phases']:
    name=mapping.get(p['name'],p['name'])
    if name in priority:segs.append((p['a'],p['b'],name))
  segs.append((client['a'],client['b'],'frontend_or_IPC'if t else 'ccache_other'))
  points=sorted({x for a,b,_ in segs for x in [a,b]})
  for a,b in zip(points,points[1:]):
   labels={k for x,y,k in segs if x<=a and y>=b}
   if labels:budget[next(k for k in priority if k in labels)]+=b-a
 elapsed=row['end_ns']/1e6-row['start_ns']/1e6
 budget['make_dispatch_and_tail']=elapsed-sum(budget.values())
 assert budget['make_dispatch_and_tail']>=-.01
 assert abs(budget['compiler_process']-sum(compiler_by_source[c['source']]['b']-compiler_by_source[c['source']]['a']for c in chain))<.01
 return dict(wall_ms=row['ms'],chain=[c['source']for c in chain],critical_chain_budget_ms=dict(budget),
             driver_processes={k:dict(count=sum(p['kind']==k for p in processes),sum_wall_ms=sum(p['b']-p['a']for p in processes if p['kind']==k),sum_cpu_ms=sum(p['cpu_ms']for p in processes if p['kind']==k))for k in ['compile','preprocess','probe']},
             max_compile_concurrency=overlap(compilers),max_backend_concurrency=overlap(backends)if backends else None,
             max_compiler_alignment_error_ms=max((p['compiler_alignment_error_ms']for p in backends),default=0))

if __name__=='__main__':
 out={}
 for case,s in [('tiny','cold'),('headers','edit-header-settled')]:
  out[case]={'timing':stats(case,s),'profiles':{}}
  for label in ['profiled','cpu-profiled']:
   if not (R/f'{label}-{case}-rows.json').exists():continue
   out[case]['profiles'][label]={arm:profile(label,case,s,arm)for arm in ['kache','ccache']}
  print(case,json.dumps(out[case],indent=2))
 save(R/'analysis.json',out)
 print('PASS: all primary/profiling outputs match raw; driver counts, daemon identity and causal critical chains checked')
 print('ANALYSIS_EXIT=0')
