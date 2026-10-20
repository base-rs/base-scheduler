#!/usr/bin/env python3
"""Count failed receives separately from the uninstrumented semantic controls."""
import fcntl
import hashlib
import importlib.util
import itertools
import json
from pathlib import Path
import random
import time
import types

root = Path.home() / '.cache/core-scheduler-single-worker-research'
helper = root / 'exit-sample-runner.py'
text = (root / 'sample-runner.py').read_text()
old = 'prefix = ["perf", "stat", "-x", ";", "-e", ",".join(events),'
assert text.count(old) == 1
helper.write_text(text.replace(old, old + '\n                  "-e", "syscalls:sys_exit_recvfrom", "--filter", "ret == -11",'))
spec = importlib.util.spec_from_file_location('helper', helper)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
output = root / 'receive-errors'
output.mkdir(exist_ok=False)
args = types.SimpleNamespace(server=root/'controls', client=Path.home()/'.cache/core-scheduler-owner-wake-final/client', output=output, privileged_perf=True)
report = {'plan': 'Two repetitions of six semantic controls at 1/32/64, perf stat with zero warmup and three-second measurement. Filter receive exits for kernel -EAGAIN (-11). Instrumented throughput is not capacity evidence.',
          'binary_sha256': hashlib.sha256(args.server.read_bytes()).hexdigest(),
          'helper_sha256': hashlib.sha256(helper.read_bytes()).hexdigest(), 'samples': []}
runs = list(itertools.product(range(1,3), ['tokio','tokio-policy-conservative','tokio-adapter','tokio-adapter-poll','iouring','iouring-poll']))
random.Random(20261025).shuffle(runs)
with open('/tmp/core-scheduler-perf.lock','a') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    report['started_utc'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
    for repeat, backend in runs:
        sample = module.sample(args,'stat',backend,1,32,64,repeat)
        report['samples'].append(sample)
        (output/'aiko.json').write_text(json.dumps(report,indent=2)+'\n')
        print(len(report['samples']),12,backend,flush=True)
    report['finished_utc'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
    (output/'aiko.json').write_text(json.dumps(report,indent=2)+'\n')
