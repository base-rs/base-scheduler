import fcntl,hashlib,json,os,shutil,subprocess
from pathlib import Path
root=Path('/home/samuel/Developer/core-rs/core-scheduler-lean-owned-read')
cache=Path('/home/samuel/.cache/core-scheduler-owned-dispatch')
source=Path('/home/samuel/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tokio-1.53.3')
copy=cache/'tokio-no-short'
env=os.environ | {'CARGO_TARGET_DIR':'/home/samuel/.cache/core-scheduler-owned-echo-diagnosis/target','CARGO_PROFILE_RELEASE_OPT_LEVEL':'3','CARGO_PROFILE_RELEASE_DEBUG':'1','CARGO_PROFILE_RELEASE_LTO':'false','CARGO_PROFILE_RELEASE_CODEGEN_UNITS':'16','CARGO_INCREMENTAL':'0','CARGO_ENCODED_RUSTFLAGS':'','RUSTFLAGS':''}
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 fcntl.flock(lock,fcntl.LOCK_EX)
 assert subprocess.check_output(['git','status','--porcelain'],cwd=root)==b''
 assert not copy.exists()
 shutil.copytree(source,copy)
 path=copy/'src/io/poll_evented.rs'
 original=path.read_text()
 assert original.count('if 0 < n && n < len {')==1
 modified=original.replace('if 0 < n && n < len {','if false && 0 < n && n < len {')
 path.write_text(modified)
 import difflib
 (cache/'tokio-no-short.patch').write_text(''.join(difflib.unified_diff(original.splitlines(True),modified.splitlines(True),fromfile='tokio-1.53.3/src/io/poll_evented.rs',tofile='tokio-1.53.3/src/io/poll_evented.rs')))
 cargo_lock=root/'Cargo.lock'
 lockfile=cargo_lock.read_bytes()
 try:
  with (cache/'logs/tokio-no-short-build.log').open('w') as out:
   result=subprocess.run(['cargo','build','--release','--offline','--config',f'patch.crates-io.tokio.path="{copy}"','-p','core-scheduler-echo-benchmark'],cwd=root,env=env,stdout=out,stderr=subprocess.STDOUT)
  if result.returncode: raise RuntimeError((cache/'logs/tokio-no-short-build.log').read_text()[-6000:])
  binary=cache/'bin/tokio-no-short'
  shutil.copy2(Path(env['CARGO_TARGET_DIR'])/'release/core-scheduler-echo-benchmark',binary)
  (cache/'tokio-no-short-build-manifest.json').write_text(json.dumps({'base':subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip(),'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'tokio_original_sha256':hashlib.sha256(original.encode()).hexdigest(),'tokio_modified_sha256':hashlib.sha256(modified.encode()).hexdigest(),'patch_sha256':hashlib.sha256((cache/'tokio-no-short.patch').read_bytes()).hexdigest(),'environment':{k:env[k] for k in ['CARGO_TARGET_DIR','CARGO_PROFILE_RELEASE_OPT_LEVEL','CARGO_PROFILE_RELEASE_DEBUG','CARGO_PROFILE_RELEASE_LTO','CARGO_PROFILE_RELEASE_CODEGEN_UNITS','CARGO_INCREMENTAL','CARGO_ENCODED_RUSTFLAGS','RUSTFLAGS']}},indent=2)+'\n')
 finally: cargo_lock.write_bytes(lockfile)
 subprocess.run(['git','diff','--exit-code'],cwd=root,check=True)
 print('built tokio-no-short',flush=True)
