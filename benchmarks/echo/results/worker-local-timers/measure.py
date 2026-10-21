import ctypes, fcntl, hashlib, importlib.util, itertools, json, os, platform, random, resource, statistics, subprocess, time
from pathlib import Path
from types import SimpleNamespace
root=Path.home()/'.cache/core-scheduler-worker-timers-production'
checkout=Path.home()/'Developer/core-rs/core-scheduler-worker-timers-integrated'
build=json.loads((root/'build.json').read_text())
spec=importlib.util.spec_from_file_location('echo_runner',checkout/'benchmarks/echo/run.py'); runner=importlib.util.module_from_spec(spec);spec.loader.exec_module(runner)
report={'build':build,'platform':platform.platform(),'runner_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'warmup_seconds':1,'measurement_seconds':2,'seed':20261101,'timer_slack_ns':1,'echo':[],'timers':[]}
def save():
 p=root/'measurements.json.tmp';p.write_text(json.dumps(report,indent=2)+'\n');p.replace(root/'measurements.json')
def binary(variant,name):return Path(build['builds'][variant]['binaries'][name]['path'])
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 print('Waiting for measurement lock',flush=True);fcntl.flock(lock,fcntl.LOCK_EX)
 libc=ctypes.CDLL(None,use_errno=True);assert libc.prctl(29,1,0,0,0)==0
 report['started_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());save()
 cases=[(1,1,64),(1,32,64),(4,128,64),(4,128,4096)]
 runs=list(itertools.product(range(1,4),['before','after'],['iouring','iouring-poll','tokio'],cases));random.Random(report['seed']).shuffle(runs)
 for repeat,variant,backend,(workers,connections,size) in runs:
  args=SimpleNamespace(server_cpus=list(range(workers)),client_cpus=[4,5,6,7],client_binary=binary('before','core-scheduler-echo-benchmark'),client_workers=4,warmup=1,seconds=2)
  data=runner.sample(binary(variant,'core-scheduler-echo-benchmark'),backend,workers,connections,size,args);data.update(variant=variant,repeat=repeat);report['echo'].append(data);save()
  print('echo',len(report['echo']),len(runs),variant,backend,workers,connections,size,flush=True)
 cases=[(1,1,10,False),(4,1024,1000,False),(4,1024,1000,True)]
 runs=list(itertools.product(range(1,4),['before','after'],cases));random.Random(report['seed']+1).shuffle(runs)
 for repeat,variant,(workers,tasks,delay,aligned) in runs:
  env=dict(os.environ,CORE_TIMER_WORK='0');env.pop('CORE_TIMER_ALIGNED',None)
  if aligned:env['CORE_TIMER_ALIGNED']='1'
  before=resource.getrusage(resource.RUSAGE_CHILDREN);started=time.monotonic()
  child=subprocess.run(['taskset','-c',','.join(map(str,range(workers))),str(binary(variant,'timers')),'iouring',str(workers),str(tasks),str(delay)],env=env,capture_output=True,text=True,check=True,timeout=30)
  after=resource.getrusage(resource.RUSAGE_CHILDREN)
  data=json.loads(child.stdout);data.update(variant=variant,repeat=repeat,aligned=aligned,user_seconds=after.ru_utime-before.ru_utime,system_seconds=after.ru_stime-before.ru_stime,lifecycle_seconds=time.monotonic()-started)
  report['timers'].append(data);save();print('timers',len(report['timers']),len(runs),variant,workers,tasks,aligned,flush=True)
 report['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());save()
 print('Complete',flush=True)
