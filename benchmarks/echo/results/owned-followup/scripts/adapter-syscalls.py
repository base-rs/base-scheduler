import fcntl, importlib.util, sys
from pathlib import Path
root=Path('/home/samuel/Developer/core-rs/core-scheduler-lean-owned-read')
cache=Path('/home/samuel/.cache/core-scheduler-owned-dispatch')
spec=importlib.util.spec_from_file_location('affinity',root/'benchmarks/echo/results/idle-search/wait-affinity.py')
module=importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
module.EVENTS['socket']=['syscalls:sys_enter_read','syscalls:sys_enter_recvfrom','syscalls:sys_enter_write','syscalls:sys_enter_sendto','syscalls:sys_enter_epoll_wait','syscalls:sys_enter_epoll_pwait','syscalls:sys_enter_futex']
sys.argv=['wait-affinity','--client','/home/samuel/.cache/core-scheduler-single-worker-completions/bin/baseline','--output',str(cache/'adapter-syscalls.json'),'--cases','1:32:64','--repeats','3','--counters','socket']
for backend in ['tokio','tokio-adapter','tokio-adapter-monitor']:
 sys.argv+=['--variant',f'{backend}={backend}={cache / "bin/lean-final"}']
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 fcntl.flock(lock,fcntl.LOCK_EX)
 module.main()
