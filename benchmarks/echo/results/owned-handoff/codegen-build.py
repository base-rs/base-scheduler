import fcntl,hashlib,json,os,shutil,subprocess
from pathlib import Path
root=Path('/home/samuel/Developer/core-rs/core-scheduler-owned-wake')
cache=Path('/home/samuel/.cache/core-scheduler-owned-dispatch')
environment=os.environ|{'CARGO_TARGET_DIR':'/home/samuel/.cache/core-scheduler-owned-echo-diagnosis/target',
 'CARGO_PROFILE_RELEASE_OPT_LEVEL':'3','CARGO_PROFILE_RELEASE_DEBUG':'1','CARGO_INCREMENTAL':'0','CARGO_ENCODED_RUSTFLAGS':'','RUSTFLAGS':''}
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 fcntl.flock(lock,fcntl.LOCK_EX)
 assert subprocess.check_output(['git','status','--porcelain'],cwd=root)==b''
 manifest={'base':subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip(), 'rustc':subprocess.check_output(['rustc','-Vv'],text=True), 'variants':{}}
 for name,lto,units in [('thin-lto','thin','16'),('cgu-one','false','1')]:
  env=environment|{'CARGO_PROFILE_RELEASE_LTO':lto,'CARGO_PROFILE_RELEASE_CODEGEN_UNITS':units}
  log=cache/'logs'/('build-'+name+'.log')
  with log.open('w') as output:
   result=subprocess.run(['cargo','build','--release','--locked','-p','core-scheduler-echo-benchmark'],cwd=root,env=env,stdout=output,stderr=subprocess.STDOUT)
  if result.returncode: raise RuntimeError(log.read_text()[-8000:])
  binary=cache/'bin'/name
  shutil.copy2(Path(env['CARGO_TARGET_DIR'])/'release/core-scheduler-echo-benchmark',binary)
  manifest['variants'][name]={'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'environment':{k:v for k,v in env.items() if k.startswith(('CARGO_','RUSTFLAGS'))}}
  (cache/'codegen-build-manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
  print('built',name,flush=True)
