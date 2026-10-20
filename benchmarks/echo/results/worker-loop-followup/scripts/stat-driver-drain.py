#!/usr/bin/env python3
import fcntl, hashlib, importlib.util, itertools, json, random, time, types
from pathlib import Path
root=Path.home()/'.cache/core-scheduler-perf-followup'
helper=root/'sample-runner.py'
spec=importlib.util.spec_from_file_location('helper',helper);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
build=json.loads((root/'driver-drain-build.json').read_text()); binaries={**build['initial_build']['binaries'],**build['binaries']}; client=Path.home()/'.cache/core-scheduler-owner-wake-final/client'
fingerprint=lambda p:{'path':str(p),'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
cases=[('before','iouring'),('driver-drain','iouring'),('before','iouring-poll'),('driver-drain','iouring-poll'),('before','tokio')]
for label in ['before','driver-drain']: assert fingerprint(Path(binaries[label]['path']))['sha256']==binaries[label]['sha256']
report={'build':build,'client':fingerprint(client),'scripts':{p.name:fingerprint(p) for p in [Path(__file__),helper,root/'supervise.py']},'samples':[],'policy':'Exclusive lock; CPU0 server, clients4-7; perfstat separate from capacity controls;0warmup,3s.'}
runs=list(itertools.product(range(1,3),cases));random.Random(20261031).shuffle(runs)
with open('/tmp/core-scheduler-perf.lock','a') as lock:
    print('Waiting for driver stat lock',flush=True);fcntl.flock(lock,fcntl.LOCK_EX)
    for repeat,(label,backend) in runs:
        output=root/'driver-drain-stat'/label;output.mkdir(parents=True,exist_ok=True)
        args=types.SimpleNamespace(server=Path(binaries[label]['path']),client=client,output=output,privileged_perf=True)
        sample=m.sample(args,'stat',backend,1,1,64,repeat);sample['variant']=label;report['samples'].append(sample)
        (root/'driver-drain-stat.json').write_text(json.dumps(report,indent=2)+'\n')
        print(len(report['samples']),len(runs),label,backend,round(sample['requests_per_second']),flush=True)
    report['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());(root/'driver-drain-stat.json').write_text(json.dumps(report,indent=2)+'\n')
