import ctypes,os,subprocess,sys
from pathlib import Path
home=Path.home();root=home/'.cache/core-scheduler-timer-ready';scripts=home/'.cache/core-scheduler-timer-ready-scripts';mode=sys.argv[1]
libc=ctypes.CDLL(None,use_errno=True);assert libc.prctl(29,ctypes.c_ulong(1),0,0,0)==0
command=['python3',str(scripts/'run.py'),'--output',str(root/mode),'--client',str(home/'.cache/core-scheduler-owner-wake-final/client'),'--build',str(root/'immediate-build.json'),'--repeats','3']
if 'mixed' in mode:
 os.environ['CORE_TIMER_TASKS']='32'
 command+=['--workload','1,32,64','--workload','4,128,64']
else:command+=['--workload','1,1,64','--workload','4,128,64']
for variant in (['immediate','ready-batch','ready-local'] if mode.startswith('optimized') else ['immediate','ready','batch']):
 binary=root/(variant+('-mixed' if 'mixed' in mode else '-core-scheduler-echo-benchmark'))
 command+=['--case',','.join([variant,'iouring-poll',str(binary)])]
if 'mixed' not in mode:command+=['--case',','.join(['control','tokio',str(root/'immediate-core-scheduler-echo-benchmark')])]
subprocess.run(command,check=True)
