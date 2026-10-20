import subprocess
from pathlib import Path
root=Path.home()/'.cache/core-scheduler-local-timers-final'
command=['python3',str(Path.home()/'.cache/core-scheduler-single-worker-research/run.py'),'--output',str(root/'echo'),'--client',str(Path.home()/'.cache/core-scheduler-owner-wake-final/client'),'--build',str(root/'build.json'),'--repeats','3']
for variant in ['before','local']:
 for backend in ['iouring','iouring-poll','tokio']:
  command+=['--case',','.join([variant,backend,str(root/(variant+'-core-scheduler-echo-benchmark'))])]
subprocess.run(command,check=True)
