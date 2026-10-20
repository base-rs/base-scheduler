#!/usr/bin/env python3
import ctypes, fcntl, hashlib, itertools, json, os, platform, random, resource, statistics, subprocess, time
from pathlib import Path
root=Path.home()/'.cache/core-scheduler-perf-followup'
while not (root/'timer-build.json').exists() or len(json.loads((root/'timer-build.json').read_text())['variants'])<4:time.sleep(1)
build=json.loads((root/'timer-build.json').read_text())
output=root/'timers';output.mkdir(exist_ok=False)
variants=['production','flush','local','local-fast']
workloads=[(1,1,10,False),(4,1024,1000,False),(4,1024,1000,True)]
report={'build':build,'runner_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'platform':platform.platform(),'seed':20261031,'warmup_seconds':1,'measurement_seconds':3,'timer_slack_ns':1,'cpu_scope':'whole process lifecycle including warmup, startup and shutdown','cpus':'0 through workers-1','samples':[]}
def save():
    p=output/'results.json.tmp';p.write_text(json.dumps(report,indent=2)+'\n');p.replace(output/'results.json')
with open('/tmp/core-scheduler-perf.lock','a') as lock:
    print('Waiting for timer measurement lock',flush=True);fcntl.flock(lock,fcntl.LOCK_EX)
    libc=ctypes.CDLL(None,use_errno=True);assert libc.prctl(29,1,0,0,0)==0
    assert Path('/proc/self/timerslack_ns').read_text().strip()=='1'
    for variant in variants:
        b=build['variants'][variant]['binaries']['timers'];assert hashlib.sha256(Path(b['path']).read_bytes()).hexdigest()==b['sha256']
    runs=list(itertools.product(range(1,4),variants,workloads));random.Random(report['seed']).shuffle(runs)
    report['started_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());save()
    for repeat,variant,(workers,tasks,delay,aligned) in runs:
        env=dict(os.environ);env.pop('CORE_TIMER_ALIGNED',None);env['CORE_TIMER_WORK']='0'
        if aligned:env['CORE_TIMER_ALIGNED']='1'
        command=['taskset','-c',','.join(map(str,range(workers))),build['variants'][variant]['binaries']['timers']['path'],'iouring',str(workers),str(tasks),str(delay)]
        before=resource.getrusage(resource.RUSAGE_CHILDREN);started=time.monotonic()
        child=subprocess.run(command,env=env,capture_output=True,text=True,timeout=30,check=True)
        after=resource.getrusage(resource.RUSAGE_CHILDREN);assert not child.stderr,child.stderr
        result=json.loads(child.stdout);result.update(variant=variant,repeat=repeat,aligned=aligned,lifecycle_seconds=time.monotonic()-started,user_seconds=after.ru_utime-before.ru_utime,system_seconds=after.ru_stime-before.ru_stime,voluntary_switches=after.ru_nvcsw-before.ru_nvcsw,involuntary_switches=after.ru_nivcsw-before.ru_nivcsw)
        report['samples'].append(result);save()
        print(len(report['samples']),len(runs),variant,workers,tasks,aligned,round(result['expiries_per_second']),round(result['user_seconds']+result['system_seconds'],4),flush=True)
    report['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());save()
    for workload in workloads:
        for variant in variants:
            samples=[s for s in report['samples'] if (s['variant'],s['workers'],s['tasks'],s['delay_us'],s['aligned'])==(variant,*workload)]
            print('RESULT',workload,variant,{k:statistics.median(s[k] for s in samples) for k in ['expiries_per_second','lateness_p50_us','lateness_p99_us']}|{'cpu':statistics.median(s['user_seconds']+s['system_seconds'] for s in samples)},flush=True)
