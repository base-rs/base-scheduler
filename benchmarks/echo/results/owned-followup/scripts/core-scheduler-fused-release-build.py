import difflib, fcntl, hashlib, json, os, shutil, subprocess
from pathlib import Path
root=Path('/home/samuel/Developer/core-rs/core-scheduler-lean-owned-read')
cache=Path('/home/samuel/.cache/core-scheduler-owned-dispatch')
source=Path('/home/samuel/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/async-task-4.7.1')
copy=cache/'async-task-fused-release'
env=os.environ | {'CARGO_TARGET_DIR':'/home/samuel/.cache/core-scheduler-owned-echo-diagnosis/target','CARGO_PROFILE_RELEASE_OPT_LEVEL':'3','CARGO_PROFILE_RELEASE_DEBUG':'1','CARGO_PROFILE_RELEASE_LTO':'false','CARGO_PROFILE_RELEASE_CODEGEN_UNITS':'16','CARGO_INCREMENTAL':'0','CARGO_ENCODED_RUSTFLAGS':'','RUSTFLAGS':''}
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 fcntl.flock(lock,fcntl.LOCK_EX)
 assert subprocess.check_output(['git','status','--porcelain'],cwd=root)==b''
 if not copy.exists(): shutil.copytree(source,copy)
 path=copy/'src/raw.rs';original=(source/'src/raw.rs').read_text()
 old='''                    let new = if state & CLOSED != 0 {
                        state & !RUNNING & !SCHEDULED
                    } else {
                        state & !RUNNING
                    };'''
 new='''                    let release = state & (CLOSED | SCHEDULED) == 0;
                    let new = if state & CLOSED != 0 {
                        state & !RUNNING & !SCHEDULED
                    } else if release {
                        (state & !RUNNING) - REFERENCE
                    } else {
                        state & !RUNNING
                    };'''
 assert original.count(old)==1
 modified=original.replace(old,new)
 old='''                            } else {
                                // Drop the task reference.
                                Self::drop_ref(ptr);
                            }
                            break;'''
 new='''                            } else {
                                // The idle transition already released this runnable's reference.
                                if new & !(REFERENCE - 1) == 0 && new & TASK == 0 {
                                    Self::destroy(ptr);
                                }
                            }
                            break;'''
 assert modified.count(old)==1
 modified=modified.replace(old,new)
 path.write_text(modified)
 patch=cache/'async-task-fused-release.patch'
 patch.write_text(''.join(difflib.unified_diff(original.splitlines(True),modified.splitlines(True),fromfile='async-task-4.7.1/src/raw.rs',tofile='async-task-4.7.1/src/raw.rs')))
 lockpath=root/'Cargo.lock';oldlock=lockpath.read_bytes()
 try:
  for name,command,cwd in [('upstream-tests',['cargo','test','--manifest-path',str(copy/'Cargo.toml'),'--lib'],copy),('native-tests',['cargo','test','--offline','--config',f'patch.crates-io.async-task.path="{copy}"','-p','core-scheduler-native','--lib'],root),('build',['cargo','build','--release','--offline','--config',f'patch.crates-io.async-task.path="{copy}"','-p','core-scheduler-echo-benchmark'],root)]:
   log=cache/'logs'/('fused-release-'+name+'.log')
   with log.open('w') as out:
    result=subprocess.run(command,cwd=cwd,env=env,stdout=out,stderr=subprocess.STDOUT)
   if result.returncode: raise RuntimeError(log.read_text()[-6000:])
   print(name,'passed',flush=True)
  binary=cache/'bin/async-task-fused-release'
  shutil.copy2(Path(env['CARGO_TARGET_DIR'])/'release/core-scheduler-echo-benchmark',binary)
  (cache/'fused-release-build-manifest.json').write_text(json.dumps({'base':subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip(),'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'upstream_original_sha256':hashlib.sha256(original.encode()).hexdigest(),'upstream_modified_sha256':hashlib.sha256(modified.encode()).hexdigest(),'patch_sha256':hashlib.sha256(patch.read_bytes()).hexdigest(),'environment':{k:v for k,v in env.items() if k.startswith(('CARGO_','RUSTFLAGS'))}},indent=2)+'\n')
 finally: lockpath.write_bytes(oldlock)
 subprocess.run(['git','diff','--exit-code'],cwd=root,check=True)
