import hashlib
import importlib.util
import itertools
import json
import platform
import random
import statistics
import subprocess
from pathlib import Path
from types import SimpleNamespace

root = Path('/home/samuel/Developer/core-rs/core-scheduler-private-io')
spec = importlib.util.spec_from_file_location('echo_run', root / 'benchmarks/echo/run.py')
run = importlib.util.module_from_spec(spec)
spec.loader.exec_module(run)
before = Path('/home/samuel/.cache/core-scheduler-interface-cleanup/echo-39a61fa')
after = Path('/home/samuel/.cache/core-scheduler-private-io/build/release/core-scheduler-echo-benchmark')
client = Path('/home/samuel/Developer/core-rs/core-scheduler-monoio-echo/target/release/core-scheduler-echo-benchmark')
variants = {
    'before-native-borrowed': (before, 'iouring-monitor'),
    'after-native-borrowed': (after, 'iouring-monitor'),
    'before-adapter-borrowed': (before, 'tokio-adapter-monitor'),
    'after-adapter-borrowed': (after, 'tokio-adapter-monitor'),
    'before-native-owned': (before, 'iouring'),
    'after-native-owned': (after, 'iouring'),
    'before-adapter-owned': (before, 'tokio-adapter'),
    'after-adapter-owned': (after, 'tokio-adapter'),
    'tokio': (before, 'tokio'),
}
args = SimpleNamespace(server_cpus=[0, 1, 2, 3], client_cpus=[4, 5, 6, 7],
                       client_binary=client, client_workers=4, warmup=1, seconds=3)
cases = [(1, 1, 64), (1, 32, 64), (4, 32, 64)]
source = hashlib.sha256()
for path in sorted(root.rglob('*')):
    if path.is_file() and not any(part in {'.git', 'target', '.agents'} for part in path.parts) and (path.suffix == '.rs' or path.name in {'Cargo.toml', 'Cargo.lock'}):
        source.update(str(path.relative_to(root)).encode())
        source.update(path.read_bytes())
report = {
    'baseline_commit': '39a61fa44adb17f7de10447a3b036b6e4847ea19',
    'source_sha256': source.hexdigest(),
    'environment': {'host': platform.node(), 'kernel': platform.release(),
                    'rustc': subprocess.check_output(['rustc', '-Vv'], text=True),
                    'cpu': subprocess.check_output(['lscpu'], text=True)},
    'settings': vars(args) | {'client_binary': str(client)},
    'variants': {k: {'binary': str(p), 'backend': b,
                     'sha256': hashlib.sha256(p.read_bytes()).hexdigest()}
                 for k, (p, b) in variants.items()},
    'client_sha256': hashlib.sha256(client.read_bytes()).hexdigest(),
    'cases': cases,
    'samples': [],
}
path = Path('/home/samuel/.cache/core-scheduler-private-io/final-echo.json')
jobs = list(itertools.product(range(4), cases, variants))
random.Random(20261024).shuffle(jobs)
for repeat, (workers, connections, size), variant in jobs:
    binary, backend = variants[variant]
    sample = run.sample(binary, backend, workers, connections, size, args)
    report['samples'].append(dict(sample, repeat=repeat, variant=variant))
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(variant, workers, connections, repeat, round(sample['requests_per_second']), flush=True)
for case in cases:
    medians = {v: statistics.median(s['requests_per_second'] for s in report['samples']
               if s['variant'] == v and (s['workers'], s['connections'], s['bytes']) == case)
               for v in variants}
    changes = {name: (medians['after-' + name] / medians['before-' + name] - 1) * 100
               for name in ['native-borrowed', 'adapter-borrowed', 'native-owned', 'adapter-owned']}
    print(case, medians, changes, flush=True)
