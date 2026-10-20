#!/usr/bin/env python3
import fcntl, hashlib, itertools, json, os, random, shutil, statistics, subprocess, time
from pathlib import Path
home=Path.home(); root=home/'.cache/core-scheduler-perf-followup'; checkout=home/'Developer/core-rs/core-scheduler-perf-followup'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
# Wait without holding the lock until the preceding build publishes all four variants.
while not (root/'timer-build.json').exists() or len(json.loads((root/'timer-build.json').read_text())['variants']) < 4: time.sleep(1)
with open('/tmp/core-scheduler-perf.lock','a') as lock:
    print('Waiting for microbenchmark build lock',flush=True);fcntl.flock(lock,fcntl.LOCK_EX)
    example=checkout/'crates/timers/examples/queue_perf.rs'; example.parent.mkdir(exist_ok=True)
    example.write_bytes((root/'queue_perf.rs').read_bytes())
    patches={k:home/'.cache'/v for k,v in [('flush','core-scheduler-timer-flush-production.patch'),('local','core-scheduler-local-timers-final/candidate.patch'),('pop','core-scheduler-timer-local-pop.patch')]}
    report={'source':subprocess.check_output(['git','rev-parse','HEAD'],cwd=checkout,text=True).strip(),'harness_sha256':sha(example),'seed':20261030,'black_box':'mutable queue reference and returned value','builds':{},'samples':[]}
    env=dict(os.environ,CARGO_TARGET_DIR=str(root/'target'))
    for label,apply,features in [('production',[],['driver']),('flush',['flush'],['driver']),('local',['local'],['driver','local']),('local-fast',['local','flush','pop'],['driver','local'])]:
        for key in apply: subprocess.run(['git','apply',str(patches[key])],cwd=checkout,check=True)
        command=['cargo','build','--release','--locked','-p','core-scheduler-timers','--features',','.join(features),'--example','queue_perf']
        with (root/(label+'-micro-build.log')).open('w') as log:subprocess.run(command,cwd=checkout,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
        binary=root/(label+'-queue-perf');shutil.copy2(root/'target/release/examples/queue_perf',binary)
        report['builds'][label]={'path':str(binary),'sha256':sha(binary),'command':command,'patches':apply}
        for key in reversed(apply):subprocess.run(['git','apply','-R',str(patches[key])],cwd=checkout,check=True)
    runs=list(itertools.product(range(1,10),['production','flush','local','local-fast'],['deadline','expiry']));random.Random(report['seed']).shuffle(runs)
    for repeat,variant,workload in runs:
        queue='local' if variant.startswith('local') else 'shared'
        command=['taskset','-c','0',report['builds'][variant]['path'],queue,workload]
        result=json.loads(subprocess.check_output(command,text=True));result.update(repeat=repeat,variant=variant)
        report['samples'].append(result)
    (root/'timer-micro.json').write_text(json.dumps(report,indent=2)+'\n')
    for queue,pair in [('shared',['production','flush']),('local',['local','local-fast'])]:
        for workload in ['deadline','expiry']:
            m={v:statistics.median(s['elapsed_ns'] for s in report['samples'] if s['variant']==v and s['workload']==workload) for v in pair}
            print(queue,workload,m,'change',m[pair[1]]/m[pair[0]]-1,flush=True)
