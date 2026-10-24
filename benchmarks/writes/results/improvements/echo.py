import fcntl,importlib.util,json,time,random,hashlib,subprocess
from pathlib import Path
from types import SimpleNamespace
root=Path('/home/samuel/Developer/core-rs/core-scheduler-ordered-writer')
base_root=Path('/home/samuel/Developer/core-rs/core-scheduler-ordered-writer-benchmark')
base=Path('/home/samuel/.cache/core-scheduler-ordered-writer-benchmark/original-echo-server')
final=root/'target/release/core-scheduler-echo-benchmark'
client=Path('/home/samuel/Developer/core-rs/core-scheduler-monoio-echo/target/release/core-scheduler-echo-benchmark')
spec=importlib.util.spec_from_file_location('echo',root/'benchmarks/echo/run.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
args=SimpleNamespace(server_cpus=[0,1,2,3],client_cpus=[4,5,6,7],client_binary=client,client_workers=4,warmup=1,seconds=3)
out=Path('/home/samuel/.cache/core-scheduler-ordered-writer-improved-echo.json')
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 fcntl.flock(lock,fcntl.LOCK_EX)
 r={'metadata':{'sources':{str(p):subprocess.check_output(['git','rev-parse','HEAD'],cwd=p,text=True).strip() for p in [root,base_root]},'binary_hashes':{str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in [base,final,client]},'baseline_source':'78846c3','final_source':'72fdd77','started_unix':time.time(),'warmup':1,'seconds':3,'repetitions':3},'samples':[]}
 entries=[(rep,label,binary,b,c) for rep in range(3) for label,binary in [('base',base),('final',final)] for b in ['iouring','tokio'] for c in [1,32]]
 random.Random(20261026).shuffle(entries)
 for idx,(rep,label,binary,b,c) in enumerate(entries):
  print(idx+1,len(entries),label,b,c,flush=True)
  x=m.sample(binary,b,1,c,64,args);x.update(variant=label,repetition=rep);r['samples'].append(x);out.write_text(json.dumps(r,indent=2)+'\n')
 r['metadata']['finished_unix']=time.time();out.write_text(json.dumps(r,indent=2)+'\n')
