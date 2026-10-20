#!/usr/bin/env python3
import fcntl, hashlib, json, os, shutil, subprocess, time
from pathlib import Path
home=Path.home();root=home/'.cache/core-scheduler-perf-followup';checkout=home/'Developer/core-rs/core-scheduler-driver-turn'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
with open('/tmp/core-scheduler-perf.lock','a') as lock:
    print('Waiting for driver drain build lock',flush=True);fcntl.flock(lock,fcntl.LOCK_EX)
    patch=root/'driver-wait-drain.patch';subprocess.run(['git','apply',str(patch)],cwd=checkout,check=True)
    env=dict(os.environ,PATH=str(home/'.cargo/bin')+':'+os.environ['PATH'],CARGO_TARGET_DIR=str(root/'driver-target'))
    report={'initial_build':json.loads((root/'driver-build.json').read_text()),'incremental_patch_sha256':sha(patch),'binaries':{}}
    for package in ['core-scheduler-native','core-scheduler-iouring']:
        subprocess.run(['cargo','test','--locked','-p',package,'--all-features'],cwd=checkout,env=env,check=True,timeout=180)
    command=['cargo','build','--release','--locked','-p','core-scheduler-echo-benchmark']
    subprocess.run(command,cwd=checkout,env=env,check=True)
    dest=root/'driver-drain-echo';shutil.copy2(root/'driver-target/release/core-scheduler-echo-benchmark',dest)
    report['binaries']['driver-drain']={'path':str(dest),'sha256':sha(dest),'command':command}
    report['tests']='Linux Native and io_uring all features passed';report['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime())
    (root/'driver-drain-build.json').write_text(json.dumps(report,indent=2)+'\n')
    print('Driver drain built and tests passed',flush=True)
