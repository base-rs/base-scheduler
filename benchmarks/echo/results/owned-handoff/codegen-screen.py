import fcntl
from pathlib import Path
import subprocess

root = Path('/home/samuel/Developer/core-rs/core-scheduler-owned-dispatch')
cache = Path('/home/samuel/.cache/core-scheduler-owned-dispatch')
command = ['python3', str(root / 'benchmarks/echo/results/idle-search/wait-affinity.py'),
    '--client', '/home/samuel/.cache/core-scheduler-single-worker-completions/bin/baseline',
    '--output', str(cache / 'codegen-screen.json'), '--cases', '1:1:64', '1:32:64', '--repeats', '5']
for build in ['lazy-quiescence', 'thin-lto', 'cgu-one']:
    for backend in ['iouring', 'tokio']:
        command += ['--variant', f'{build}-{backend}={backend}={cache / "bin" / build}']
with open('/tmp/core-scheduler-perf.lock', 'a') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    subprocess.run(command, cwd=root, check=True)
