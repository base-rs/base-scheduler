import fcntl
from pathlib import Path
import subprocess

root = Path('/home/samuel/Developer/core-rs/core-scheduler-lean-owned-read')
cache = Path('/home/samuel/.cache/core-scheduler-owned-dispatch')
command = ['python3', str(root / 'benchmarks/echo/results/idle-search/wait-affinity.py'),
    '--client', '/home/samuel/.cache/core-scheduler-single-worker-completions/bin/baseline',
    '--output', str(cache / 'lean-cpu.json'), '--cases', '1:32:64', '--repeats', '3', '--counters', 'cpu']
for name, backend, binary in [('baseline', 'iouring', 'lazy-quiescence'),
        ('candidate', 'iouring', 'lean-final'), ('tokio', 'tokio', 'lean-final')]:
    command += ['--variant', f'{name}={backend}={cache / "bin" / binary}']
with open('/tmp/core-scheduler-perf.lock', 'a') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    subprocess.run(command, cwd=root, check=True)
