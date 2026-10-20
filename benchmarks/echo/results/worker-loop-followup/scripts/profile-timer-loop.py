#!/usr/bin/env python3
"""Compare frozen production and worker-local timer binaries without rebuilding."""
import ctypes, fcntl, hashlib, importlib.util, itertools, json, platform, random, subprocess, time, types
from pathlib import Path

root = Path.home() / '.cache/core-scheduler-perf-followup'
previous = Path.home() / '.cache/core-scheduler-local-timers-final'
helper = root / 'sample-runner.py'
spec = importlib.util.spec_from_file_location('helper', helper)
module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
build = json.loads((previous / 'build.json').read_text())
client = Path.home() / '.cache/core-scheduler-owner-wake-final/client'
fingerprint = lambda p: {'path':str(p),'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
assert fingerprint(client)['sha256'] == 'db6c914ab75011477ec6232e681046b1e75532cbaa1c81663bac984591675cf3'
binaries = {}
for label in ['before', 'local']:
    binaries[label] = previous / (label + '-core-scheduler-echo-benchmark')
    assert fingerprint(binaries[label])['sha256'] == build['binaries'][label]['core-scheduler-echo-benchmark']['sha256']
libc=ctypes.CDLL(None, use_errno=True)
assert libc.prctl(29, 1, 0, 0, 0) == 0
report = {'build':build, 'binaries':{k:fingerprint(v) for k,v in binaries.items()},
          'client':fingerprint(client), 'scripts':{p.name:fingerprint(p) for p in [Path(__file__),helper,root/'supervise.py']},
          'timerslack_ns':Path('/proc/self/timerslack_ns').read_text().strip(),
          'platform':platform.platform(), 'perf_version':subprocess.check_output(['perf','--version'],text=True).strip(),
          'policy':'Exclusive lock; server CPU 0; client CPUs 4-7; instrumented results are diagnostic only.',
          'samples':[]}
assert report['timerslack_ns']=='1'
def save():
    target=root/'timer-loop-profile.json'; tmp=target.with_suffix('.tmp')
    tmp.write_text(json.dumps(report,indent=2)+'\n'); tmp.replace(target)
with open('/tmp/core-scheduler-perf.lock','a') as lock:
    print('Waiting for exclusive performance lock.',flush=True)
    fcntl.flock(lock,fcntl.LOCK_EX)
    report['started_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());save()
    for mode,repeats in [('control',3),('stat',2),('perf',2)]:
        runs=list(itertools.product(range(1,repeats+1),binaries,['iouring','iouring-poll']))
        random.Random(20261029+repeats).shuffle(runs)
        for repeat,label,backend in runs:
            output=root/'timer-loop'/label/mode; output.mkdir(parents=True,exist_ok=True)
            args=types.SimpleNamespace(server=binaries[label],client=client,output=output,privileged_perf=mode!='control',perf_event='cycles')
            result=module.sample(args,mode,backend,1,1,64,repeat)
            result['variant']=label;report['samples'].append(result);save()
            print(len(report['samples']),28,mode,label,backend,round(result['requests_per_second']),flush=True)
    for result in report['samples']:
        if result['mode']!='perf': continue
        data=root/'timer-loop'/result['variant']/'perf'/(result['identifier']+'.data')
        for kind,flags in [('self',['--no-children','-g','none']),('callgraph',['--children','-g','graph,0.5,caller'])]:
            with data.with_suffix('.'+kind+'.txt').open('w') as out:
                subprocess.run(['sudo','-n','perf','report','--stdio','--header','--show-nr-samples','--sort','symbol','--percent-limit','0.2',*flags,'-i',str(data)],stdout=out,check=True)
    report['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());save()
