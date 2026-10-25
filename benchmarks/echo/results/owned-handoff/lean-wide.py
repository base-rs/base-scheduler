import fcntl
from pathlib import Path
import subprocess

root = Path('/home/samuel/Developer/core-rs/core-scheduler-lean-owned-read')
cache = Path('/home/samuel/.cache/core-scheduler-owned-dispatch')
command = ['python3', str(root / 'benchmarks/echo/results/idle-search/wait-affinity.py'),
    '--client', '/home/samuel/.cache/core-scheduler-single-worker-completions/bin/baseline',
    '--output', str(cache / 'lean-wide.json'), '--cases',
    '1:1:64', '1:32:64', '4:1:64', '4:32:64',
    '1:1:4096', '1:32:4096', '4:1:4096', '4:32:4096', '--repeats', '3']
for name, backend, binary in [
    ('baseline-owned', 'iouring', 'lazy-quiescence'),
    ('candidate-owned', 'iouring', 'lean-final'),
    ('baseline-borrowed', 'iouring-monitor', 'lazy-quiescence'),
    ('candidate-borrowed', 'iouring-monitor', 'lean-final'),
    ('tokio', 'tokio', 'lean-final'),
    ('tokio-adapter-owned', 'tokio-adapter', 'lean-final'),
    ('tokio-adapter-borrowed', 'tokio-adapter-monitor', 'lean-final'),
]:
    command += ['--variant', f'{name}={backend}={cache / "bin" / binary}']
with open('/tmp/core-scheduler-perf.lock', 'a') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    subprocess.run(command, cwd=root, check=True)
