#!/usr/bin/env python3
import fcntl, os, subprocess
from pathlib import Path
home=Path.home(); root=home/'.cache/core-scheduler-perf-followup'; checkout=home/'Developer/core-rs/core-scheduler-driver-turn'
with open('/tmp/core-scheduler-perf.lock','a') as lock:
    print('Waiting for driver coverage lock',flush=True);fcntl.flock(lock,fcntl.LOCK_EX)
    env=dict(os.environ,PATH=str(home/'.cargo/bin')+':'+os.environ['PATH'],CARGO_TARGET_DIR=str(root/'driver-coverage-target'),LLVM_COV=str(root/'llvm-cov-wrapper.py'),LLVM_PROFDATA='/usr/bin/llvm-profdata')
    for package in ['core-scheduler-native','core-scheduler-iouring']:
        subprocess.run(['cargo','bake','test:coverage','--package',package,'--all-features','true'],cwd=checkout,env=env,check=True)
    subprocess.run(['cargo','fmt','--all','--','--check'],cwd=checkout,env=env,check=True)
    subprocess.run(['cargo','clippy','--locked','-p','core-scheduler-native','-p','core-scheduler-iouring','--all-targets','--all-features','--','-D','warnings'],cwd=checkout,env=env,check=True)
    print('Driver coverage and lint passed',flush=True)
