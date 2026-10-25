import fcntl
from pathlib import Path
import subprocess

root = Path('/home/samuel/Developer/core-rs/core-scheduler-lean-owned-read')
cache = Path('/home/samuel/.cache/core-scheduler-owned-dispatch')
command = ['python3', str(root / 'benchmarks/echo/results/idle-search/wait-affinity.py'),
    '--client', '/home/samuel/.cache/core-scheduler-single-worker-completions/bin/baseline',
    '--output', str(cache / 'batch-screen.json'), '--cases', '1:32:64', '--repeats', '5']
for name, backend, binary in [('baseline', 'iouring', 'lean-final'), ('batch-4', 'iouring', 'batch-submission-4'),
        ('batch-16', 'iouring', 'batch-submission-16'), ('tokio', 'tokio', 'lean-final')]:
    command += ['--variant', f'{name}={backend}={cache / "bin" / binary}']
with open('/tmp/core-scheduler-perf.lock', 'a') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    subprocess.run(command, cwd=root, check=True)
