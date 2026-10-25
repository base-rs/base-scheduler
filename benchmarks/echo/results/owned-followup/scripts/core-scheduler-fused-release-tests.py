import fcntl, os, subprocess
from pathlib import Path
cache=Path('/home/samuel/.cache/core-scheduler-owned-dispatch')
env=os.environ | {'CARGO_TARGET_DIR':'/home/samuel/.cache/core-scheduler-owned-echo-diagnosis/target','RUST_TEST_THREADS':'1'}
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 fcntl.flock(lock,fcntl.LOCK_EX)
 log=cache/'logs/fused-release-upstream-integration.log'
 with log.open('w') as out:
  result=subprocess.run(['cargo','test','--locked','--manifest-path',str(cache/'async-task-fused-release/Cargo.toml'),'--tests'],cwd=cache,env=env,stdout=out,stderr=subprocess.STDOUT)
 if result.returncode: raise RuntimeError(log.read_text()[-6000:])
 print(log.read_text()[-2500:],flush=True)
