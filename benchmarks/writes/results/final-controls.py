import fcntl,importlib.util,json,time,random,subprocess,hashlib
from pathlib import Path
spec=importlib.util.spec_from_file_location('writes','benchmarks/writes/run.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
cache=Path('/home/samuel/.cache/core-scheduler-ordered-writer-benchmark');out=Path('/home/samuel/.cache/core-scheduler-ordered-writer-final-controls.json')
base=str(cache/'base-shared-prepare-server');issued=str(cache/'issued-shared-prepare-server');waitall=str(cache/'waitall-shared-prepare-server');final=str(Path('target/release/core-scheduler-write-benchmark').resolve());client=str(cache/'initial-server')
cases=[(1,1,64,1,0,0),(1,1,65536,8,8,0),(1,1,262144,8,0,65536),(4,32,262144,8,8,65536)]
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 fcntl.flock(lock,fcntl.LOCK_EX)
 entries=[(i,'final-'+b,b,final,c) for i in range(3) for c in cases for b in ['tokio','tokio-monitor','tokio-writer','iouring-monitor','iouring-writer']]
 entries += [(i,label+'-'+b,b,binary,(1,1,65536,8,8,0)) for i in range(3) for label,binary in [('base',base),('issued',issued),('waitall',waitall)] for b in ['tokio','iouring-monitor']]
 random.Random(20261025).shuffle(entries)
 r={'metadata':{'git_head':m.command('git','rev-parse','HEAD'),'git_status':m.command('git','status','--short'),'started_unix':time.time(),'warmup':1,'seconds':3,'repetitions':3,'cpu_affinity':'servers0/0-3; frozenclient4-7','sources':{},'binary_hashes':{p:m.digest(p) for p in [base,issued,waitall,final,client]}},'samples':[]};m.save(out,r)
 for idx,(rep,label,backend,binary,case) in enumerate(entries):
  print(idx+1,len(entries),label,case,flush=True)
  x={'repetition':rep,'started_unix':time.time()};x.update(m.sample(binary,client,backend,case,1,3));x['server_backend']=backend;x['backend']=label;r['samples'].append(x);m.save(out,r)
 r['metadata']['finished_unix']=time.time();m.save(out,r)
