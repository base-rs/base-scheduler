import fcntl, hashlib, json, os, random, shutil, subprocess
from pathlib import Path
root=Path('/home/samuel/Developer/core-rs/core-scheduler-lean-owned-read')
cache=Path('/home/samuel/.cache/core-scheduler-owned-dispatch')
env=os.environ | {'CARGO_TARGET_DIR':'/home/samuel/.cache/core-scheduler-owned-echo-diagnosis/target','CARGO_PROFILE_RELEASE_OPT_LEVEL':'3','CARGO_PROFILE_RELEASE_DEBUG':'1','CARGO_PROFILE_RELEASE_LTO':'false','CARGO_PROFILE_RELEASE_CODEGEN_UNITS':'16','CARGO_INCREMENTAL':'0','CARGO_ENCODED_RUSTFLAGS':'','RUSTFLAGS':''}
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 fcntl.flock(lock,fcntl.LOCK_EX)
 assert subprocess.check_output(['git','status','--porcelain'],cwd=root)==b''
 path=root/'benchmarks/echo/src/bin/task-poll.rs'
 assert not path.exists()
 path.parent.mkdir(exist_ok=True)
 try:
  path.write_bytes((cache/'core-scheduler-task-poll.rs').read_bytes())
  with (cache/'logs/task-poll-current-build.log').open('w') as out:
   result=subprocess.run(['cargo','build','--release','--locked','-p','core-scheduler-echo-benchmark','--bin','task-poll'],cwd=root,env=env,stdout=out,stderr=subprocess.STDOUT)
  if result.returncode: raise RuntimeError((cache/'logs/task-poll-current-build.log').read_text()[-6000:])
  binary=cache/'bin/task-poll-current'
  shutil.copy2(Path(env['CARGO_TARGET_DIR'])/'release/task-poll',binary)
  report={'base':subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip(),'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'source_sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'server_cpus':[0],'environment':{k:v for k,v in env.items() if k.startswith(('CARGO_','RUSTFLAGS'))},'samples':[]}
 finally: path.unlink()
 subprocess.run(['git','diff','--exit-code'],cwd=root,check=True)
 jobs=[(backend,count,wake) for backend in ['native','tokio'] for count in [1,32] for wake in ['borrowed','owned']]
 random.Random(20261025).shuffle(jobs)
 for backend,count,wake in jobs:
  sample=json.loads(subprocess.check_output(['taskset','-c','0',str(binary),backend,'1',str(count),'5000000','5','1'],env=env|{'TASK_POLL_WAKE':wake},text=True))
  report['samples'].append(sample)
  (cache/'task-poll-current.json').write_text(json.dumps(report,indent=2)+'\n')
  import statistics
  rate=statistics.median(x['polls_per_second'] for x in sample['samples'])
  print(backend,count,wake,round(rate),flush=True)
