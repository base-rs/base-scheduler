#!/usr/bin/env python3
import fcntl,hashlib,json,os,shutil,subprocess,time
from pathlib import Path
root=Path.home()/'.cache/core-scheduler-worker-timers'
checkout=Path.home()/'Developer/core-rs/core-scheduler-worker-timers'
sha=lambda path:hashlib.sha256(path.read_bytes()).hexdigest()
report={'source':subprocess.check_output(['git','rev-parse','HEAD'],cwd=checkout,text=True).strip(),'patch_sha256':sha(root/'candidate.patch'),'harness_sha256':sha(root/'timers.rs'),'rustc':subprocess.check_output(['rustc','-Vv'],text=True),'command':'cargo build --release --locked -p core-scheduler-echo-benchmark --bins','binaries':{}}
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 fcntl.flock(lock,fcntl.LOCK_EX)
 assert not subprocess.check_output(['git','status','--porcelain'],cwd=checkout,text=True)
 directory=checkout/'benchmarks/echo/src/bin';directory.mkdir(exist_ok=True)
 shutil.copy2(root/'timers.rs',directory/'timers.rs')
 for variant in ['before','integrated']:
  if variant=='integrated':subprocess.run(['git','apply','-'],input=(root/'candidate.patch').read_bytes(),cwd=checkout,check=True)
  with (root/(variant+'-build.log')).open('w') as output:
   subprocess.run(['cargo','build','--release','--locked','-p','core-scheduler-echo-benchmark','--bins'],cwd=checkout,stdout=output,stderr=subprocess.STDOUT,check=True)
  report['binaries'][variant]={}
  for name in ['core-scheduler-echo-benchmark','timers']:
   dest=root/(variant+'-'+name);shutil.copy2(checkout/'target/release'/name,dest)
   report['binaries'][variant][name]={'path':str(dest),'sha256':sha(dest)}
  print('Built',variant,flush=True)
 with (root/'linux-native-tests.log').open('w') as output:
  subprocess.run(['cargo','test','--locked','-p','core-scheduler-native','--lib'],cwd=checkout,stdout=output,stderr=subprocess.STDOUT,check=True)
 report['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime())
 (root/'build.json').write_text(json.dumps(report,indent=2)+'\n')
print('Builds and Linux tests complete.',flush=True)
