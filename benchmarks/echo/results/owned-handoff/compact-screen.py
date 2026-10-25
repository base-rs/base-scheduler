import fcntl
from pathlib import Path
import subprocess

root = Path('/home/samuel/Developer/core-rs/core-scheduler-owned-dispatch')
cache = Path('/home/samuel/.cache/core-scheduler-owned-dispatch')
command = ['python3', str(root / 'benchmarks/echo/results/idle-search/wait-affinity.py'),
    '--client', '/home/samuel/.cache/core-scheduler-single-worker-completions/bin/baseline',
    '--output', str(cache / 'compact-exclusive-screen.json'), '--cases', '1:1:64', '1:32:64', '--repeats', '5']
for name, backend, binary in [
    ('baseline', 'iouring', 'lazy-quiescence'),
    ('compact-ahead', 'iouring', 'compact-ahead'),
    ('compact-cold', 'iouring', 'compact-cold'),
    ('exclusive-admission', 'iouring', 'exclusive-admission'),
    ('ownership', 'iouring', 'ownership'),
    ('exclusive-ownership', 'iouring', 'exclusive-ownership'),
    ('tokio', 'tokio', 'lazy-quiescence'),
]:
    command += ['--variant', f'{name}={backend}={cache / "bin" / binary}']
with open('/tmp/core-scheduler-perf.lock', 'a') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    subprocess.run(command, cwd=root, check=True)
