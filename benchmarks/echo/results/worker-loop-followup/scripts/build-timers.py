#!/usr/bin/env python3
import fcntl, hashlib, json, os, shutil, subprocess, time
from pathlib import Path
home=Path.home(); root=home/'.cache/core-scheduler-perf-followup'
checkout=home/'Developer/core-rs/core-scheduler-perf-followup'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def run(args,**kw): return subprocess.run(args,cwd=checkout,check=True,**kw)
with open('/tmp/core-scheduler-perf.lock','a') as lock:
    print('Waiting for build lock',flush=True);fcntl.flock(lock,fcntl.LOCK_EX)
    if not checkout.exists():
        subprocess.run(['git','-C',str(home/'Developer/core-rs/core-scheduler'),'worktree','add','--detach',str(checkout),'64eb1548e1371b067f0e7c8b85d5a37c2f745768'],check=True)
    assert not subprocess.check_output(['git','status','--porcelain'],cwd=checkout)
    scripts=home/'.cache/core-scheduler-timer-ready-scripts'
    bindir=checkout/'benchmarks/echo/src/bin';bindir.mkdir(exist_ok=True)
    (bindir/'timers.rs').write_bytes((scripts/'timers.rs').read_bytes())
    flush=home/'.cache/core-scheduler-timer-flush-production.patch'
    local=home/'.cache/core-scheduler-local-timers-final/candidate.patch'
    pop=home/'.cache/core-scheduler-timer-local-pop.patch'
    env=dict(os.environ,CARGO_TARGET_DIR=str(root/'target'))
    report={'source':subprocess.check_output(['git','rev-parse','HEAD'],cwd=checkout,text=True).strip(),
      'rustc':subprocess.check_output(['rustc','-Vv'],text=True),
      'environment_overrides':{k:v for k,v in os.environ.items() if k.startswith(('RUST','CARGO_PROFILE'))},
      'harness_sha256':sha(bindir/'timers.rs'),'patches':{p.name:sha(p) for p in [flush,local,pop]},'variants':{}}
    for label,patches,features in [('production',[],[]),('flush',[flush],[]),('local',[local],['core-scheduler-native/worker-timers']),('local-fast',[local,flush,pop],['core-scheduler-native/worker-timers'])]:
        for patch in patches:run(['git','apply',str(patch)])
        command=['cargo','build','--release','--locked','-p','core-scheduler-echo-benchmark','--bins']
        if features: command+=['--features',','.join(features)]
        with (root/(label+'-build.log')).open('w') as log:run(command,env=env,stdout=log,stderr=subprocess.STDOUT)
        binaries={}
        for name in ['timers','core-scheduler-echo-benchmark']:
            dest=root/(label+'-'+name);shutil.copy2(root/'target/release'/name,dest)
            binaries[name]={'path':str(dest),'sha256':sha(dest)}
        report['variants'][label]={'command':command,'patches':[p.name for p in patches],'binaries':binaries,
          'diff_sha256':hashlib.sha256(subprocess.check_output(['git','diff'],cwd=checkout)).hexdigest()}
        for patch in reversed(patches):run(['git','apply','-R',str(patch)])
        report['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime())
        (root/'timer-build.json').write_text(json.dumps(report,indent=2)+'\n')
        print('Built',label,flush=True)
