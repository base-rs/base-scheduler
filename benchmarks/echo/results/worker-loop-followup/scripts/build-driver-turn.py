#!/usr/bin/env python3
import fcntl, hashlib, json, os, shutil, subprocess, time
from pathlib import Path
home=Path.home(); root=home/'.cache/core-scheduler-perf-followup'; checkout=home/'Developer/core-rs/core-scheduler-driver-turn'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
with open('/tmp/core-scheduler-perf.lock','a') as lock:
    print('Waiting for driver experiment build lock',flush=True);fcntl.flock(lock,fcntl.LOCK_EX)
    if not checkout.exists():subprocess.run(['git','-C',str(home/'Developer/core-rs/core-scheduler'),'worktree','add','--detach',str(checkout),'64eb1548e1371b067f0e7c8b85d5a37c2f745768'],check=True)
    assert not subprocess.check_output(['git','status','--porcelain'],cwd=checkout)
    common=root/'common-hotpath.patch'
    subprocess.run(['git','apply',str(common)],cwd=checkout,check=True)
    env=dict(os.environ,PATH=str(home/'.cargo/bin')+':'+os.environ['PATH'],CARGO_TARGET_DIR=str(root/'driver-target'))
    patch=root/'driver-turn-prepared.patch'
    report={'source':subprocess.check_output(['git','rev-parse','HEAD'],cwd=checkout,text=True).strip(),'patch_sha256':sha(patch),'common_patch_sha256':sha(common),'rustc':subprocess.check_output(['rustc','-Vv'],text=True),'environment_overrides':{k:v for k,v in os.environ.items() if k.startswith(('RUST','CARGO_PROFILE'))},'binaries':{}}
    for variant in ['before','driver-turn']:
        if variant!='before':subprocess.run(['git','apply',str(patch)],cwd=checkout,check=True)
        command=['cargo','build','--release','--locked','-p','core-scheduler-echo-benchmark']
        with (root/(variant+'-driver-build.log')).open('w') as log:subprocess.run(command,cwd=checkout,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
        dest=root/(variant+'-driver-echo');shutil.copy2(root/'driver-target/release/core-scheduler-echo-benchmark',dest)
        report['binaries'][variant]={'path':str(dest),'sha256':sha(dest),'command':command}
        (root/'driver-build.json').write_text(json.dumps(report,indent=2)+'\n')
        print('Built',variant,flush=True)
    with (root/'driver-tests.log').open('w') as log:
        for package in ['core-scheduler-native','core-scheduler-iouring']:
            subprocess.run(['cargo','test','--locked','-p',package,'--all-features'],cwd=checkout,env=env,stdout=log,stderr=subprocess.STDOUT,check=True,timeout=180)
    report['tests']='Linux Native and io_uring all features passed'
    report['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());(root/'driver-build.json').write_text(json.dumps(report,indent=2)+'\n')
    print('Driver tests passed',flush=True)
