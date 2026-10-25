import fcntl, importlib.util, subprocess, sys
from pathlib import Path
root=Path('/home/samuel/Developer/core-rs/core-scheduler-lean-owned-read')
cache=Path('/home/samuel/.cache/core-scheduler-owned-dispatch')
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 fcntl.flock(lock,fcntl.LOCK_EX)
 module_path=root/'benchmarks/echo/results/idle-search/wait-affinity.py'
 spec=importlib.util.spec_from_file_location('wait_affinity',module_path)
 runner=importlib.util.module_from_spec(spec); spec.loader.exec_module(runner)
 events=['cycles:u','instructions:u','L1-dcache-loads:u','L1-dcache-load-misses:u','L1-icache-load-misses:u']
 runner.EVENTS['cache']=events
 for mode in [None,'cache']:
  output=cache/('concurrency-cache.json' if mode else 'concurrency-screen.json')
  if mode:
   check=subprocess.run(['sudo','-n','perf','stat','-x,','-e',','.join(events),'--','true'],capture_output=True,text=True)
   (cache/'logs/cache-events-check.log').write_text(check.stdout+check.stderr)
   if check.returncode or '<not supported>' in check.stderr:
    raise RuntimeError(check.stdout+check.stderr)
  sys.argv=[str(module_path),'--client','/home/samuel/.cache/core-scheduler-single-worker-completions/bin/baseline','--output',str(output),'--cases','1:1:64','1:8:64','1:32:64','1:128:64','--repeats',str(2 if mode else 3),'--variant',f'native=iouring={cache / "bin/lean-final"}','--variant',f'tokio=tokio={cache / "bin/lean-final"}']
  if mode: sys.argv+=['--counters',mode]
  runner.main()
