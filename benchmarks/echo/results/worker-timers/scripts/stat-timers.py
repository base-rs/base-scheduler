#!/usr/bin/env python3
import fcntl,hashlib,itertools,json,os,platform,random,subprocess,time
from pathlib import Path
root=Path.home()/'.cache/core-scheduler-worker-timers';out=root/'timer-stat';out.mkdir(exist_ok=False)
events=['task-clock','cycles','instructions','context-switches','syscalls:sys_enter_io_uring_enter','syscalls:sys_enter_futex','syscalls:sys_enter_read','syscalls:sys_enter_write']
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
report={'build':json.loads((root/'build.json').read_text()),'runner_sha256':sha(Path(__file__)),'events':events,'instrumented':True,'denominator':'whole process; timer histogram excludes warmup and final late expiries','samples':[]}
runs=list(itertools.product(range(1,3),['before','integrated'],[(1,1,1000),(4,1,1000),(4,1024,1000)]));random.Random(20261021).shuffle(runs)
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 print('Waiting for exclusive stat lock.',flush=True);fcntl.flock(lock,fcntl.LOCK_EX)
 for repeat,variant,(workers,tasks,delay) in runs:
  identifier=f'{variant}-w{workers}-t{tasks}-d{delay}-r{repeat}'
  command=['sudo','-n','perf','stat','-x',';','-e',','.join(events),'-o',str(out/(identifier+'.stat')),'--','setpriv',f'--reuid={os.getuid()}',f'--regid={os.getgid()}','--init-groups','--','taskset','-c',','.join(map(str,range(workers))),str(root/(variant+'-timers')),'iouring',str(workers),str(tasks),str(delay)]
  result=subprocess.run(command,capture_output=True,text=True,timeout=30,check=True)
  assert not result.stderr,result.stderr
  sample=json.loads(result.stdout);sample.update(variant=variant,repeat=repeat,identifier=identifier)
  report['samples'].append(sample);(out/'results.json').write_text(json.dumps(report,indent=2)+'\n')
  print(identifier,flush=True)
print('Complete.',flush=True)
