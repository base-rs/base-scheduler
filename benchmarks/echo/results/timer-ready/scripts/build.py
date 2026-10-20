import fcntl,hashlib,json,os,shutil,subprocess,sys,time
from pathlib import Path
home=Path.home();root=home/'.cache/core-scheduler-timer-ready';checkout=home/'Developer/core-rs/core-scheduler-timer-ready';scripts=home/'.cache/core-scheduler-timer-ready-scripts'
root.mkdir(exist_ok=True)
variant=sys.argv[1];sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 print('Waiting for build lock',variant,flush=True);fcntl.flock(lock,fcntl.LOCK_EX)
 if variant!='immediate':subprocess.run(['git','apply',str(root/(variant+'.patch'))],cwd=checkout,check=True)
 bindir=checkout/'benchmarks/echo/src/bin';bindir.mkdir(exist_ok=True);shutil.copy2(scripts/'timers.rs',bindir/'timers.rs')
 original=(checkout/'benchmarks/echo/src/server.rs').read_bytes()
 env=dict(os.environ,CARGO_TARGET_DIR=str(root/'target'))
 features='core-scheduler-native/'+('ready-timer-local' if variant=='ready-local' else 'ready-timers' if variant.startswith('ready') else 'worker-timers')
 command=['cargo','build','--release','--locked','-p','core-scheduler-echo-benchmark','--bins','--features',features]
 report={'variant':variant,'source':subprocess.check_output(['git','rev-parse','HEAD'],cwd=checkout,text=True).strip(),'rustc':subprocess.check_output(['rustc','-Vv'],text=True),'build_command':command,'environment_overrides':{k:v for k,v in os.environ.items() if k.startswith(('RUST','CARGO_PROFILE'))},'files':{},'binaries':{}}
 try:
  for mode in ['ordinary','mixed']:
   if mode=='mixed':(checkout/'benchmarks/echo/src/server.rs').write_bytes((scripts/'mixed-server.rs').read_bytes())
   with (root/(variant+'-'+mode+'-build.log')).open('w') as log:subprocess.run(command,cwd=checkout,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
   for name in (['core-scheduler-echo-benchmark','timers'] if mode=='ordinary' else ['core-scheduler-echo-benchmark']):
    dest=root/(variant+'-'+('mixed' if mode=='mixed' else name));shutil.copy2(root/'target/release'/name,dest);report['binaries'][dest.name]={'path':str(dest),'sha256':sha(dest)}
   print('Built',variant,mode,flush=True)
 finally:(checkout/'benchmarks/echo/src/server.rs').write_bytes(original)
 for path in ['crates/native/src/worker.rs','crates/native/src/task_service.rs','crates/native/src/worker_timers.rs','crates/timers/src/local.rs','benchmarks/echo/src/bin/timers.rs']:
  p=checkout/path
  if p.exists():report['files'][path]=sha(p)
 if variant!='immediate':
  with (root/(variant+'-tests.log')).open('w') as log:
   subprocess.run(['cargo','test','--locked','-p','core-scheduler-native','--all-features','--all-targets'],cwd=checkout,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
   subprocess.run(['cargo','test','--locked','-p','core-scheduler-iouring','--all-features'],cwd=checkout,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
  subprocess.run(['git','apply','-R',str(root/(variant+'.patch'))],cwd=checkout,check=True)
 report['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime())
 (root/(variant+'-build.json')).write_text(json.dumps(report,indent=2)+'\n')
 print('Finished',variant,flush=True)
