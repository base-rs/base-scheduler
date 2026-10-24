import fcntl,importlib.util,json,random,time
from pathlib import Path
spec=importlib.util.spec_from_file_location('writes','benchmarks/writes/run.py');mod=importlib.util.module_from_spec(spec);spec.loader.exec_module(mod)
p=Path('/home/samuel/.cache/core-scheduler-ordered-writer-shared-prepare.json')
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 fcntl.flock(lock,fcntl.LOCK_EX)
 r=json.loads(p.read_text());m=r['metadata'];completed={(x['repetition'],x['backend'],tuple(x.get(v,x['result'].get(v)) for v in ['workers','connections','bytes','chunks','work','send_buffer'])) for x in r['samples'] if 'error' not in x}
 assert mod.digest(m['binary'])==m['binary_sha256'];assert mod.digest(m['client_binary'])==m['client_binary_sha256']
 for v in m['variants'].values():assert mod.digest(v['binary'])==v['binary_sha256']
 schedule=[(i,b,tuple(c)) for i in range(m['repetitions']) for c in m['cases'] for b in m['backends']];random.Random(m['seed']).shuffle(schedule)
 m.setdefault('resumed_unix',[]).append(time.time())
 for i,b,c in schedule:
  if (i,b,c) in completed:continue
  print(i,b,c,flush=True);v=m['variants'].get(b,{'backend':b,'binary':m['binary']});x={'repetition':i,'started_unix':time.time()};x.update(mod.sample(v['binary'],m['client_binary'],v['backend'],c,m['warmup'],m['seconds']));x['server_backend']=v['backend'];x['backend']=b;r['samples'].append(x);mod.save(p,r)
 m['finished_unix']=time.time();mod.save(p,r)
