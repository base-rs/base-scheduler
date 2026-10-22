#!/usr/bin/env python3
"""Compare preserved baseline and balanced-admission binaries on Aiko."""

import hashlib
import importlib.util
import itertools
import json
import os
from pathlib import Path
import random
import re
import subprocess
from types import SimpleNamespace

ROOT = Path('/home/samuel/.cache/core-scheduler-task-admission')
OUT = Path('/home/samuel/.cache/core-scheduler-task-admission-results')
spec = importlib.util.spec_from_file_location('echo_runner', ROOT / 'benchmarks/echo/run.py')
echo = importlib.util.module_from_spec(spec)
spec.loader.exec_module(echo)
seed = 20261022
repeats = 7
hashes = {name: hashlib.sha256((OUT / name).read_bytes()).hexdigest()
          for name in ('echo-baseline', 'echo-balanced', 'pools-baseline', 'pools-balanced')}
manifest = dict(seed=seed, repeats=repeats, binaries=hashes,
                baseline='6b9f8a3', pools_base='c461fe8ff281d6d694c390d0f97d0096df730534',
                runtime_patch=(Path('/home/samuel/.cache/core-scheduler-task-admission-runtime.patch').read_text()),
                pool_diagnostic_patch='benchmarks/buffers/experiments/receive-cost/balance.patch at 5e745a22f10f1e5f084c813f8454daf1be1a4d2a',
                kernel=subprocess.check_output(['uname', '-a'], text=True).strip(),
                rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(),
                topology=subprocess.check_output(['lscpu', '-e=CPU,CORE,SOCKET,NODE,CACHE'], text=True),
                load_before=os.getloadavg())
(OUT / 'provenance.json').write_text(json.dumps(manifest, indent=2) + '\n')
rng = random.Random(seed)
args = SimpleNamespace(server_cpus=[0, 1, 2, 3], client_cpus=[4, 5, 6, 7],
                       client_binary=OUT / 'echo-baseline', client_workers=4,
                       warmup=0.5, seconds=1.5)
with (OUT / 'echo.jsonl').open('w') as output:
    for repeat in range(repeats):
        cases = list(itertools.product((1, 4), (1, 32), (64, 4096)))
        rng.shuffle(cases)
        for workers, connections, size in cases:
            variants = ['baseline', 'balanced']
            rng.shuffle(variants)
            for variant in variants:
                result = echo.sample(OUT / f'echo-{variant}', 'iouring-monitor', workers, connections, size, args)
                result.update(repeat=repeat, variant=variant, size=size,
                              server_sha256=hashes[f'echo-{variant}'], client_sha256=hashes['echo-baseline'])
                output.write(json.dumps(result) + '\n')
                output.flush()
                print('echo', repeat, workers, connections, size, variant, round(result['requests_per_second']), flush=True)

with (OUT / 'pools.jsonl').open('w') as output:
    for repeat in range(repeats):
        cases = list(itertools.product((1, 2, 4), ('total', 'worker-equal')))
        rng.shuffle(cases)
        for workers, layout in cases:
            variants = ['baseline', 'balanced']
            rng.shuffle(variants)
            for variant in variants:
                binary = OUT / f'pools-{variant}'
                command = ['taskset', '-c', '2-15', str(binary), 'iouring', 'pool', '65536',
                           '30000', str(workers), 'tcp', '8', 'receive', '8', layout]
                p = subprocess.run(command, text=True, capture_output=True, timeout=90, check=True)
                result = json.loads(p.stdout)
                assert result['bytes_received'] == 65536 * 30000 * 8 * 8
                owners = [int(value) for value in re.findall(r'RECEIVE_STATISTICS worker=(\d+)', p.stderr)]
                if layout == 'worker-equal':
                    assert len(owners) == 8, p.stderr
                result.update(repeat=repeat, variant=variant, args=command,
                              sha256=hashes[f'pools-{variant}'], diagnostics=p.stderr,
                              receivers_per_worker=[owners.count(worker) for worker in range(workers)])
                output.write(json.dumps(result) + '\n')
                output.flush()
                print('pools', repeat, workers, layout, variant,
                      round(result['messages_per_second']), result['receivers_per_worker'], flush=True)
manifest['load_after'] = os.getloadavg()
(OUT / 'provenance.json').write_text(json.dumps(manifest, indent=2) + '\n')
