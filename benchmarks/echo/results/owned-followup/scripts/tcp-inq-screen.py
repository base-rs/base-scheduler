import fcntl
from pathlib import Path
import subprocess

root = Path('/home/samuel/Developer/core-rs/core-scheduler-lean-owned-read')
cache = Path('/home/samuel/.cache/core-scheduler-owned-dispatch')
command = ['python3', str(root / 'benchmarks/echo/results/idle-search/wait-affinity.py'),
    '--client', '/home/samuel/.cache/core-scheduler-single-worker-completions/bin/baseline',
    '--output', str(cache / 'tcp-inq-screen.json'), '--cases', '1:1:64', '1:32:64', '4:32:64', '--repeats', '5']
for name, backend, binary in [
        ('baseline-owned', 'tokio-adapter', 'lean-final'),
        ('candidate-owned', 'tokio-adapter', 'tcp-inq-readiness'),
        ('baseline-borrowed', 'tokio-adapter-monitor', 'lean-final'),
        ('candidate-borrowed', 'tokio-adapter-monitor', 'tcp-inq-readiness'),
        ('tokio', 'tokio', 'lean-final')]:
    command += ['--variant', f'{name}={backend}={cache / "bin" / binary}']
with open('/tmp/core-scheduler-perf.lock', 'a') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    subprocess.run(command, cwd=root, check=True)
