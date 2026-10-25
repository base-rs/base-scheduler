import fcntl, hashlib, importlib.util, json, os, shlex, subprocess, tempfile
from pathlib import Path
from types import SimpleNamespace

root = Path('/home/samuel/Developer/core-rs/core-scheduler-lean-owned-read')
cache = Path('/home/samuel/.cache/core-scheduler-owned-dispatch')
out = cache / 'adapter-profiles'
spec = importlib.util.spec_from_file_location('echo_run', root / 'benchmarks/echo/run.py')
run = importlib.util.module_from_spec(spec)
spec.loader.exec_module(run)
settings = SimpleNamespace(server_cpus=[0], client_cpus=[4,5,6,7],
    client_binary=Path('/home/samuel/.cache/core-scheduler-single-worker-completions/bin/baseline'),
    client_workers=4, warmup=1, seconds=8)
with open('/tmp/core-scheduler-perf.lock', 'a') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    out.mkdir(exist_ok=True)
    report = {'event': 'cycles:u', 'frequency': 1999, 'seconds': 8, 'warmup': 1,
        'note': 'Profile-instrumented rates are not capacity measurements.', 'samples': []}
    for backend in ['tokio', 'tokio-adapter', 'tokio-adapter-monitor']:
        binary = cache / 'bin/lean-final'
        data = out / (backend + '.data')
        with tempfile.TemporaryDirectory(prefix='core-scheduler-lean-profile-') as directory:
            wrapper = Path(directory) / 'server'
            wrapper.write_text('#!/bin/sh\nexec ' + shlex.join(['sudo','-n','perf','record','-q',
                '-e','cycles:u','-F','1999','--call-graph','dwarf','-o',str(data),'--',str(binary)]) + ' "$@"\n')
            wrapper.chmod(0o700)
            sample = run.sample(wrapper, backend, 1, 32, 64, settings)
            report['samples'].append(sample)
        for name, extra in [('self', ['--no-children']), ('callgraph', ['--children'])]:
            with (out / (backend + '-' + name + '.txt')).open('w') as output:
                subprocess.run(['sudo','-n','perf','report','--stdio','-i',str(data),
                    '--sort','symbol','--percent-limit','0.2'] + extra, stdout=output, check=True)
        report['binary_sha256'] = hashlib.sha256(binary.read_bytes()).hexdigest()
        (out / 'manifest.json').write_text(json.dumps(report, indent=2) + '\n')
        print('profiled', backend, flush=True)
