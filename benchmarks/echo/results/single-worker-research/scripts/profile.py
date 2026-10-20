#!/usr/bin/env python3
"""Profile the merged batching implementation; keep instrumentation out of capacity results."""
import fcntl
import hashlib
import importlib.util
import itertools
import json
from pathlib import Path
import platform
import random
import subprocess
import time
import types

root = Path.home() / '.cache/core-scheduler-single-worker-research'
server = Path.home() / '.cache/core-scheduler-wake-validation/fixed'
client = Path.home() / '.cache/core-scheduler-owner-wake-final/client'
helper = root / 'sample-runner.py'
spec = importlib.util.spec_from_file_location('helper', helper)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
backends = ['tokio', 'tokio-adapter', 'tokio-adapter-poll', 'iouring', 'iouring-poll']
report = {
    'merged_source': '130cd4a67e526e80ba9a5e7e7ebbe81494bd903b',
    'measured_source': '9386f7c4f6cb0dccd7e528512476d2cf963f7a88',
    'source_note': 'Frozen optimized binary from wake validation. Executable/test sources match merged implementation; later changes are documentation and evidence.',
    'binaries': {name: {'path': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}
                 for name, path in [('server', server), ('client', client)]},
    'scripts': {path.name: hashlib.sha256(path.read_bytes()).hexdigest()
                for path in [Path(__file__), helper, root / 'supervise.py']},
    'platform': platform.platform(),
    'cpu_topology': subprocess.check_output(['lscpu', '-e=CPU,CORE,SOCKET,NODE,CACHE'], text=True),
    'perf_version': subprocess.check_output(['perf', '--version'], text=True).strip(),
    'governors': {str(cpu): Path(f'/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor').read_text().strip() for cpu in range(8)},
    'seed': 20261024,
    'plan': '30 control samples (3 repeats, five backends, 1/32/64 and 4/128/64). Then 10 whole-process perf-stat samples (2 repeats, five backends, 1/32/64, zero warmup). Then five cycles profiles (five backends, 1/32/64, 1-second warmup and 5-second measurement).',
    'samples': [],
}

def save():
    temp = root / 'profile.json.tmp'
    temp.write_text(json.dumps(report, indent=2) + '\n')
    temp.replace(root / 'profile.json')

with open('/tmp/core-scheduler-perf.lock', 'a') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    report['started_utc'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
    save()
    for mode, repeats, workloads in [('control', 3, [(1,32,64),(4,128,64)]),
                                      ('stat', 2, [(1,32,64)]), ('perf', 1, [(1,32,64)])]:
        output = root / mode
        output.mkdir(exist_ok=False)
        args = types.SimpleNamespace(server=server, client=client, output=output,
                                     privileged_perf=mode != 'control', perf_event='cycles')
        cases = list(itertools.product(range(1, repeats+1), backends, workloads))
        random.Random(report['seed'] + ['control','stat','perf'].index(mode)).shuffle(cases)
        for repeat, backend, (workers, connections, size) in cases:
            sample = module.sample(args, mode, backend, workers, connections, size, repeat)
            report['samples'].append(sample)
            save()
            print(len(report['samples']), 45, mode, backend, workers, connections,
                  round(sample['requests_per_second']), flush=True)
    report['finished_utc'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
    save()

for sample in report['samples']:
    if sample['mode'] != 'perf':
        continue
    data = root / 'perf' / (sample['identifier'] + '.data')
    for kind, flags in [('self', ['--no-children', '-g', 'none']),
                        ('callgraph', ['--children', '-g', 'graph,1,caller'])]:
        with data.with_suffix('.' + kind + '.txt').open('w') as output:
            subprocess.run(['sudo', '-n', 'perf', 'report', '--stdio', '--header',
                            '--show-nr-samples', '--sort', 'symbol', '--percent-limit', '0.3',
                            *flags, '-i', str(data)], stdout=output, check=True)
