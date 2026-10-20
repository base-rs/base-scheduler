import ctypes,os,subprocess
from pathlib import Path
root=Path.home()/'.cache/core-scheduler-local-timers-final'
libc=ctypes.CDLL(None,use_errno=True);assert libc.prctl(29,ctypes.c_ulong(1),0,0,0)==0
os.environ['CORE_TIMER_TASKS']='32'
command=['python3',str(Path.home()/'.cache/core-scheduler-single-worker-research/run.py'),'--output',str(root/'mixed'),'--client',str(Path.home()/'.cache/core-scheduler-owner-wake-final/client'),'--build',str(root/'mixed-build.json'),'--repeats','3','--workload','1,32,64','--workload','4,128,64']
for variant in ['before','local']:
 for backend in ['iouring','iouring-poll']:command+=['--case',','.join([variant,backend,str(root/(variant+'-mixed'))])]
subprocess.run(command,check=True)
