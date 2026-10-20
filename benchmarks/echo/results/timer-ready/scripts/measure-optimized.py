import ctypes,fcntl,hashlib,itertools,json,os,platform,random,resource,statistics,subprocess,time
from pathlib import Path
home=Path.home();root=home/'.cache/core-scheduler-timer-ready';output=root/'optimized-timers';output.mkdir(exist_ok=False)
# Relative low-load/throughput plus synchronized expiry with useful work after waking.
workloads=[(1,1,10,False,0),(1,32,1000,False,0),(4,1024,1000,False,0),(4,1024,1000,True,0)]
variants=['immediate','ready-batch','ready-local'];runs=list(itertools.product(range(1,4),variants,workloads));random.Random(20261022).shuffle(runs)
report={'builds':{v:json.loads((root/(v+'-build.json')).read_text()) for v in variants},'runner_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'platform':platform.platform(),'samples':[],'warmup_seconds':1,'measurement_seconds':3,'timer_slack_ns':1,'cpus':'0 through workers-1','whole_lifecycle_cpu':True,'shuffle_seed':20261022}
def save():
 p=output/'results.json.tmp';p.write_text(json.dumps(report,indent=2)+'\n');p.replace(output/'results.json')
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 print('Waiting for timer measurement lock',flush=True);fcntl.flock(lock,fcntl.LOCK_EX)
 libc=ctypes.CDLL(None,use_errno=True);assert libc.prctl(29,ctypes.c_ulong(1),0,0,0)==0;assert int(Path('/proc/self/timerslack_ns').read_text())==1
 report['started_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());save()
 for repeat,variant,(workers,tasks,delay,aligned,work) in runs:
  env=dict(os.environ,CORE_TIMER_WORK=str(work));env.pop('CORE_TIMER_ALIGNED',None)
  if aligned:env['CORE_TIMER_ALIGNED']='1'
  command=['taskset','-c',','.join(map(str,range(workers))),str(root/(variant+'-timers')),'iouring',str(workers),str(tasks),str(delay)]
  before=resource.getrusage(resource.RUSAGE_CHILDREN);started=time.monotonic();child=subprocess.run(command,env=env,capture_output=True,text=True,timeout=30,check=True);after=resource.getrusage(resource.RUSAGE_CHILDREN)
  assert not child.stderr,child.stderr;result=json.loads(child.stdout)
  result.update(variant=variant,repeat=repeat,aligned=aligned,work=work,lifecycle_seconds=time.monotonic()-started,user_seconds=after.ru_utime-before.ru_utime,system_seconds=after.ru_stime-before.ru_stime,voluntary_switches=after.ru_nvcsw-before.ru_nvcsw,involuntary_switches=after.ru_nivcsw-before.ru_nivcsw)
  report['samples'].append(result);save();print(len(report['samples']),len(runs),variant,workers,tasks,aligned,work,round(result['expiries_per_second']),round(result['lateness_p99_us'],2),round(result['user_seconds']+result['system_seconds'],3),flush=True)
 report['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());save()
for workload in workloads:
 for variant in variants:
  ss=[s for s in report['samples'] if (s['variant'],s['workers'],s['tasks'],s['delay_us'],s['aligned'],s['work'])==(variant,*workload)]
  print('RESULT',workload,variant,{k:statistics.median(s[k] for s in ss) for k in ['expiries_per_second','lateness_p50_us','lateness_p99_us']}|{'cpu':statistics.median(s['user_seconds']+s['system_seconds'] for s in ss)},flush=True)
