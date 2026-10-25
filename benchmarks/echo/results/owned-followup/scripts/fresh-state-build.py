import fcntl,hashlib,json,os,shutil,subprocess
from pathlib import Path
root=Path('/home/samuel/Developer/core-rs/core-scheduler-lean-owned-read')
cache=Path('/home/samuel/.cache/core-scheduler-owned-dispatch')
source=Path('/home/samuel/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/async-task-4.7.1')
copy=cache/'async-task-fresh-state'
env=os.environ | {'CARGO_TARGET_DIR':'/home/samuel/.cache/core-scheduler-owned-echo-diagnosis/target','CARGO_PROFILE_RELEASE_OPT_LEVEL':'3','CARGO_PROFILE_RELEASE_DEBUG':'1','CARGO_PROFILE_RELEASE_LTO':'false','CARGO_PROFILE_RELEASE_CODEGEN_UNITS':'16','CARGO_INCREMENTAL':'0','CARGO_ENCODED_RUSTFLAGS':'','RUSTFLAGS':''}
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 fcntl.flock(lock,fcntl.LOCK_EX)
 assert subprocess.check_output(['git','status','--porcelain'],cwd=root)==b''
 assert not copy.exists()
 shutil.copytree(source,copy)
 path=copy/'src/raw.rs'
 original=path.read_text()
 needle = '            Poll::Pending => {\n                let mut future_dropped = false;'
 assert original.count(needle)==1
 modified=original.replace(needle, '            Poll::Pending => {\n                state = (*raw.header).state.load(Ordering::Acquire);\n                let mut future_dropped = false;')
 path.write_text(modified)
 import difflib
 (cache/'async-task-fresh-state.patch').write_text(''.join(difflib.unified_diff(original.splitlines(True),modified.splitlines(True),fromfile='async-task-4.7.1/src/raw.rs',tofile='async-task-4.7.1/src/raw.rs')))
 cargo_lock=root/'Cargo.lock'
 lockfile=cargo_lock.read_bytes()
 try:
  with (cache/'logs/async-task-fresh-state-build.log').open('w') as out:
   result=subprocess.run(['cargo','build','--release','--offline','--config',f'patch.crates-io.async-task.path="{copy}"','-p','core-scheduler-echo-benchmark'],cwd=root,env=env,stdout=out,stderr=subprocess.STDOUT)
  if result.returncode: raise RuntimeError((cache/'logs/async-task-fresh-state-build.log').read_text()[-6000:])
  binary=cache/'bin/async-task-fresh-state'
  shutil.copy2(Path(env['CARGO_TARGET_DIR'])/'release/core-scheduler-echo-benchmark',binary)
  (cache/'async-task-fresh-state-build-manifest.json').write_text(json.dumps({'base':subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip(),'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'upstream_original_sha256':hashlib.sha256(original.encode()).hexdigest(),'upstream_modified_sha256':hashlib.sha256(modified.encode()).hexdigest(),'patch_sha256':hashlib.sha256((cache/'async-task-fresh-state.patch').read_bytes()).hexdigest(),'environment':{k:env[k] for k in ['CARGO_TARGET_DIR','CARGO_PROFILE_RELEASE_OPT_LEVEL','CARGO_PROFILE_RELEASE_DEBUG','CARGO_PROFILE_RELEASE_LTO','CARGO_PROFILE_RELEASE_CODEGEN_UNITS','CARGO_INCREMENTAL','CARGO_ENCODED_RUSTFLAGS','RUSTFLAGS']}},indent=2)+'\n')
 finally: cargo_lock.write_bytes(lockfile)
 subprocess.run(['git','diff','--exit-code'],cwd=root,check=True)
 print('built async-task-fresh-state',flush=True)
