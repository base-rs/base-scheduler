import fcntl,hashlib,json,os,shutil,subprocess
from pathlib import Path
home=Path.home();root=home/'.cache/core-scheduler-timer-ready';checkout=home/'Developer/core-rs/core-scheduler-timer-ready';scripts=home/'.cache/core-scheduler-timer-ready-scripts';source=checkout/'benchmarks/echo/src/server.rs'
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 print('Waiting for mixed rebuild lock',flush=True);fcntl.flock(lock,fcntl.LOCK_EX)
 # The earlier copied source preserved an old mtime, so Cargo reused the ordinary executable.
 # No mixed samples ran; force fresh source mtimes and assert distinct executable identities.
 empty=root/'mixed';assert not list(empty.iterdir());empty.rmdir()
 original=source.read_bytes();env=dict(os.environ,CARGO_TARGET_DIR=str(root/'target'))
 try:
  for variant in ['immediate','ready','batch']:
   if variant!='immediate':subprocess.run(['git','apply','--whitespace=nowarn',str(root/(variant+'.patch'))],cwd=checkout,check=True)
   source.write_bytes((scripts/'mixed-server.rs').read_bytes())
   features='core-scheduler-native/'+('ready-timers' if variant=='ready' else 'worker-timers')
   command=['cargo','build','--release','--locked','-p','core-scheduler-echo-benchmark','--bin','core-scheduler-echo-benchmark','--features',features]
   with (root/(variant+'-mixed-rebuild.log')).open('w') as log:subprocess.run(command,cwd=checkout,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
   dest=root/(variant+'-mixed');shutil.copy2(root/'target/release/core-scheduler-echo-benchmark',dest)
   p=root/(variant+'-build.json');report=json.loads(p.read_text());sha=hashlib.sha256(dest.read_bytes()).hexdigest();assert sha!=report['binaries'][variant+'-core-scheduler-echo-benchmark']['sha256']
   report['binaries'][variant+'-mixed']={'path':str(dest),'sha256':sha};report['mixed_harness_sha256']=hashlib.sha256(source.read_bytes()).hexdigest();p.write_text(json.dumps(report,indent=2)+'\n')
   if variant!='immediate':subprocess.run(['git','apply','--whitespace=nowarn','-R',str(root/(variant+'.patch'))],cwd=checkout,check=True)
   print('Rebuilt',variant,'mixed',flush=True)
 finally:source.write_bytes(original)
