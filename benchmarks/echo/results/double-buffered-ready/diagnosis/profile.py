#!/usr/bin/env python3
"""Compare the frozen serial echo binaries; keep instrumentation separate from timing."""
import argparse
import fcntl
import hashlib
import importlib.util
import itertools
import json
from pathlib import Path
import random
import subprocess
import time
import types

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--output', required=True, type=Path)
p.add_argument('--helper', required=True, type=Path)
p.add_argument('--client', required=True, type=Path)
p.add_argument('--case', required=True, action='append', help='label,backend,binary')
p.add_argument('--mode', choices=['control', 'stat', 'perf', 'strace'], required=True)
p.add_argument('--repeats', type=int, default=3)
a = p.parse_args()
a.output.mkdir(parents=True, exist_ok=False)
spec = importlib.util.spec_from_file_location('sample', a.helper)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
cases = [x.split(',', 2) for x in a.case]
fingerprint = lambda path: hashlib.sha256(Path(path).read_bytes()).hexdigest()
report = dict(mode=a.mode, cases=[dict(label=l, backend=b, binary=f, sha256=fingerprint(f)) for l,b,f in cases],
              helper_sha256=fingerprint(a.helper), client_sha256=fingerprint(a.client), samples=[])
args = types.SimpleNamespace(client=a.client, output=a.output, privileged_perf=a.mode in ['stat', 'perf'], perf_event='cycles')
runs = list(itertools.product(range(1, a.repeats + 1), cases))
random.Random(20261022).shuffle(runs)
with open('/tmp/core-scheduler-perf.lock', 'a') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    report['started_utc'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
    for repeat, (label, backend, binary) in runs:
        args.server = Path(binary)
        # Prefix the helper's mode to keep filenames unique without changing its profiler mode.
        directory = a.output / label
        directory.mkdir(exist_ok=True)
        args.output = directory
        sample = m.sample(args, a.mode, backend, 1, 1, 64, repeat)
        sample['variant'] = label
        report['samples'].append(sample)
        (a.output / 'aiko.json').write_text(json.dumps(report, indent=2) + '\n')
        print(label, a.mode, backend, repeat, round(sample['requests_per_second']), flush=True)
    report['finished_utc'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
    (a.output / 'aiko.json').write_text(json.dumps(report, indent=2) + '\n')
if a.mode == 'perf':
    for sample in report['samples']:
        data = a.output / sample['variant'] / (sample['identifier'] + '.data')
        with data.with_suffix('.self.txt').open('w') as output:
            subprocess.run(['sudo', '-n', 'perf', 'report', '--stdio', '--header', '--no-children', '-g', 'none',
                            '--show-nr-samples', '--sort', 'symbol', '--percent-limit', '0.1', '-i', str(data)], stdout=output, check=True)
