import ctypes,fcntl,hashlib,itertools,json,platform,random,resource,statistics,subprocess,time
from pathlib import Path
root=Path.home()/'.cache/core-scheduler-local-timers-final';output=root/'timer-slack';output.mkdir(exist_ok=False)
workloads=[(1,1,10),(1,1,1000),(1,32,1000),(4,1,1000),(4,32,1000),(4,1024,1000)]
cases=[(v,'iouring') for v in ['before','local']]+[(v,'native') for v in ['before','local']]
# Readiness parking control needs only the low-load and contended cases.
runs=[(repeat,variant,backend,w,t,d) for repeat in range(1,4) for variant,backend in cases for w,t,d in workloads if backend=='iouring' or (w,t,d) in [(1,1,1000),(4,1024,1000)]]
random.Random(20261021).shuffle(runs)
report={'build':json.loads((root/'build.json').read_text()),'runner_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'platform':platform.platform(),'warmup_seconds':1,'measurement_seconds':3,'whole_lifecycle_cpu':True,'cpus':'0 through workers-1','samples':[]}
def save():
 p=output/'results.json.tmp';p.write_text(json.dumps(report,indent=2)+'\n');p.replace(output/'results.json')
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 print('Waiting for exclusive timer lock',flush=True);fcntl.flock(lock,fcntl.LOCK_EX)
 libc=ctypes.CDLL(None,use_errno=True);assert libc.prctl(29,ctypes.c_ulong(1),0,0,0)==0,ctypes.get_errno()
 report['timer_slack_ns']=int(Path('/proc/self/timerslack_ns').read_text());assert report['timer_slack_ns']==1
 report['started_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());save()
 for repeat,variant,backend,workers,tasks,delay in runs:
  command=['taskset','-c',','.join(str(c) for c in range(workers)),str(root/(variant+'-timers')),backend,str(workers),str(tasks),str(delay)]
  before=resource.getrusage(resource.RUSAGE_CHILDREN);started=time.monotonic()
  child=subprocess.Popen(command,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
  try:
   time.sleep(.25);threads=len(list(Path('/proc',str(child.pid),'task').iterdir()))
   stdout,stderr=child.communicate(timeout=20);assert child.returncode==0,stderr
  finally:
   if child.poll() is None:child.kill();child.wait()
  after=resource.getrusage(resource.RUSAGE_CHILDREN);result=json.loads(stdout);assert not stderr,stderr
  result.update(variant=variant,repeat=repeat,threads_at_250ms=threads,lifecycle_seconds=time.monotonic()-started,user_seconds=after.ru_utime-before.ru_utime,system_seconds=after.ru_stime-before.ru_stime,voluntary_switches=after.ru_nvcsw-before.ru_nvcsw,involuntary_switches=after.ru_nivcsw-before.ru_nivcsw)
  report['samples'].append(result);save()
  print(len(report['samples']),len(runs),variant,backend,workers,tasks,delay,round(result['expiries_per_second']),result['lateness_p50_us'],round(result['user_seconds']+result['system_seconds'],3),flush=True)
 report['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());save()
for backend in ['iouring','native']:
 for workload in workloads:
  for variant in ['before','local']:
   ss=[s for s in report['samples'] if (s['variant'],s['backend'],s['workers'],s['tasks'],s['delay_us'])==(variant,backend,*workload)]
   if ss:print('RESULT',backend,workload,variant,{k:statistics.median(s[k] for s in ss) for k in ['expiries_per_second','lateness_p50_us','lateness_p99_us','user_seconds','system_seconds','threads_at_250ms']},flush=True)
