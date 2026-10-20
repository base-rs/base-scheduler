import fcntl,hashlib,json,subprocess,shutil,time
from pathlib import Path
root=Path.home()/'.cache/core-scheduler-local-timers-final';checkout=Path.home()/'Developer/core-rs/core-scheduler-local-timers'
root.mkdir(exist_ok=True)
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 fcntl.flock(lock,fcntl.LOCK_EX)
 subprocess.run(['git','reset','--hard','0d2c4f1'],cwd=checkout,check=True)
 (checkout/'crates/native/src/worker_timers.rs').unlink(missing_ok=True)
 shutil.copy2(Path.home()/'.cache/local-timers-final.patch',root/'candidate.patch')
 report={'source':subprocess.check_output(['git','rev-parse','HEAD'],cwd=checkout,text=True).strip(),'patch_sha256':sha(root/'candidate.patch'),'rustc':subprocess.check_output(['rustc','-Vv'],text=True),'command':'cargo build --release --locked -p core-scheduler-echo-benchmark --bins [--features core-scheduler-native/worker-timers]','binaries':{}}
 for variant in ['before','local']:
  if variant=='local':subprocess.run(['git','apply',str(root/'candidate.patch')],cwd=checkout,check=True)
  command=['cargo','build','--release','--locked','-p','core-scheduler-echo-benchmark','--bins']
  if variant=='local':command+=['--features','core-scheduler-native/worker-timers']
  with (root/(variant+'-build.log')).open('w') as output:subprocess.run(command,cwd=checkout,stdout=output,stderr=subprocess.STDOUT,check=True)
  report['binaries'][variant]={}
  for name in ['core-scheduler-echo-benchmark','timers']:
   dest=root/(variant+'-'+name);shutil.copy2(checkout/'target/release'/name,dest);report['binaries'][variant][name]={'path':str(dest),'sha256':sha(dest)}
  print('Built',variant,flush=True)
 with (root/'linux-tests.log').open('w') as output:subprocess.run(['cargo','test','--locked','-p','core-scheduler-native','--all-features','--lib'],cwd=checkout,stdout=output,stderr=subprocess.STDOUT,check=True)
 report['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());(root/'build.json').write_text(json.dumps(report,indent=2)+'\n');print('Linux tests passed',flush=True)
