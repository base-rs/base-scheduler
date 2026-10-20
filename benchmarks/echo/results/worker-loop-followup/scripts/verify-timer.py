#!/usr/bin/env python3
import fcntl, os, subprocess
from pathlib import Path
home=Path.home();root=home/'.cache/core-scheduler-perf-followup';checkout=home/'Developer/core-rs/core-scheduler-perf-followup'
patch=home/'.cache/core-scheduler-timer-flush-production.patch'
with open('/tmp/core-scheduler-perf.lock','a') as lock:
    print('Waiting for timer verification lock',flush=True);fcntl.flock(lock,fcntl.LOCK_EX)
    subprocess.run(['git','apply',str(patch)],cwd=checkout,check=True)
    env=dict(os.environ,PATH=str(home/'.cargo/bin')+':'+os.environ['PATH'],CARGO_TARGET_DIR=str(root/'coverage-target'),LLVM_COV=str(root/'llvm-cov-wrapper.py'),LLVM_PROFDATA='/usr/bin/llvm-profdata')
    try:
        for extra in [[],['--all-features','true']]:
            subprocess.run(['cargo','bake','test:coverage','--package','core-scheduler-timers',*extra],cwd=checkout,env=env,check=True)
    finally:subprocess.run(['git','apply','-R',str(patch)],cwd=checkout,check=True)
