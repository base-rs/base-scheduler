import fcntl
import importlib.util
import json
import os
from pathlib import Path
import random
import time

CACHE = Path('/home/samuel/.cache')
spec = importlib.util.spec_from_file_location('writes', CACHE / 'core-scheduler-ordered-writer-benchmark/profile-run.py')
writes = importlib.util.module_from_spec(spec)
spec.loader.exec_module(writes)
binary = str(Path('target/release/core-scheduler-write-benchmark').resolve())
client = str(CACHE / 'core-scheduler-ordered-writer-benchmark/initial-server')
output = CACHE / 'ordered-writer-taskwork-policy-control.json'
cases = [(1, 1, 262144, 8, 0, 65536), (1, 1, 262144, 8, 8, 65536), (4, 32, 262144, 8, 8, 65536)]
seed = 20261027
with open('/tmp/core-scheduler-perf.lock', 'a') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    metadata = {
        'source_head': writes.command('git', 'rev-parse', 'HEAD'),
        'source_status': writes.command('git', 'status', '--short'),
        'binary': binary,
        'binary_sha256': writes.digest(binary),
        'client_binary': client,
        'client_sha256': writes.digest(client),
        'rustc': writes.command('rustc', '-vV'),
        'hostname': writes.command('hostname'),
        'uname': writes.command('uname', '-a'),
        'cpu': json.loads(writes.command('lscpu', '-J')),
        'governors': {str(cpu): Path(f'/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor').read_text().strip() for cpu in range(8)},
        'started_unix': time.time(),
        'cases': cases,
        'seed': seed,
        'warmup_seconds': 1,
        'measurement_seconds': 3,
        'repetitions': 3,
        'environment_switch': 'CORE_SCHEDULER_TASKWORK=normal|cooperative; only ring constructor reads it',
        'taskrun_flag_check': 'Enabled in both modes, preserving identical code layout and executable hash.',
        'patch_path': str(CACHE / 'ordered-writer-taskwork-policy-control.patch'),
        'patch_sha256': writes.digest(CACHE / 'ordered-writer-taskwork-policy-control.patch'),
    }
    report = {'metadata': metadata, 'samples': []}
    schedule = [(rep, mode, case) for rep in range(3) for mode in ['normal', 'cooperative'] for case in cases]
    random.Random(seed).shuffle(schedule)
    metadata['schedule'] = schedule
    writes.save(output, report)
    for index, (rep, mode, case) in enumerate(schedule):
        print(f'{index + 1}/{len(schedule)} {mode} {case} repetition={rep}', flush=True)
        os.environ['CORE_SCHEDULER_TASKWORK'] = mode
        sample = writes.sample(binary, client, 'iouring-writer', case, 1, 3)
        sample.update({'backend': mode + '-writer', 'server_backend': 'iouring-writer', 'taskwork_policy': mode, 'server_environment': {'CORE_SCHEDULER_TASKWORK': mode}, 'repetition': rep})
        report['samples'].append(sample)
        writes.save(output, report)
    assert writes.digest(binary) == metadata['binary_sha256']
    metadata['finished_unix'] = time.time()
    writes.save(output, report)
