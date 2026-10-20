import ctypes,fcntl,hashlib,itertools,json,os,random,subprocess
from pathlib import Path
home=Path.home();root=home/'.cache/core-scheduler-timer-ready';out=root/'stat';out.mkdir(exist_ok=False)
events=['task-clock','cycles','instructions','context-switches','syscalls:sys_enter_io_uring_enter','syscalls:sys_enter_futex','syscalls:sys_enter_read','syscalls:sys_enter_write']
report={'runner_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'events':events,'instrumented':True,'denominator':'whole process; histogram excludes warmup and final late expiries','samples':[]}
runs=list(itertools.product(range(1,3),['immediate','ready','batch']));random.Random(20261022).shuffle(runs)
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 print('Waiting for stat lock',flush=True);fcntl.flock(lock,fcntl.LOCK_EX)
 libc=ctypes.CDLL(None,use_errno=True);assert libc.prctl(29,ctypes.c_ulong(1),0,0,0)==0;assert int(Path('/proc/self/timerslack_ns').read_text())==1
 for repeat,variant in runs:
  identifier=f'{variant}-r{repeat}'
  command=['sudo','-n','perf','stat','-x',';','-e',','.join(events),'-o',str(out/(identifier+'.stat')),'--','setpriv',f'--reuid={os.getuid()}',f'--regid={os.getgid()}','--init-groups','--','taskset','-c','0-3',str(root/(variant+'-timers')),'iouring','4','1024','1000']
  result=subprocess.run(command,capture_output=True,text=True,timeout=30,check=True);assert not result.stderr,result.stderr
  sample=json.loads(result.stdout);sample.update(variant=variant,repeat=repeat,identifier=identifier)
  report['samples'].append(sample);(out/'results.json').write_text(json.dumps(report,indent=2)+'\n');print(identifier,flush=True)
