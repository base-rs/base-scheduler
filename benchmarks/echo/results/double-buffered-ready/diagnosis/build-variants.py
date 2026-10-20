#!/usr/bin/env python3
"""Build isolated ablations of the batch policy, never changing the reference checkout."""
import fcntl
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

checkout, output = map(Path, sys.argv[1:])
output.mkdir(parents=True, exist_ok=True)
source_path = checkout / 'crates/native/src/worker.rs'
original = subprocess.check_output(['git', 'show', 'b03e582:crates/native/src/worker.rs'], cwd=checkout, text=True)
swap = '            std::mem::swap(&mut local.ready, &mut local.current);\n            !local.current.is_empty()'
singleton = '''            if local.ready.is_empty() {
                false
            } else {
                std::mem::swap(&mut local.ready, &mut local.current);
                true
            }'''
external = '''    if local.ready.is_empty() || iteration % 61 == 0 {
        for queue in [&shared.ready, incoming] {
            let _ = take_stolen(queue.steal_batch(&local.ready), &mut retry);
        }
    }'''
admission = '''    for queue in [&shared.ready, incoming] {
        let _ = take_stolen(queue.steal_batch(&local.ready), &mut retry);
    }'''
assert swap in original and admission in original
variants = {
    'singleton': original.replace(swap, singleton),
    'external': original.replace(admission, external),
    'both': original.replace(swap, singleton).replace(admission, external),
}
builds = {}
with open('/tmp/core-scheduler-perf.lock', 'a') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    for label, source in variants.items():
        source_path.write_text(source)
        (output / (label + '.patch')).write_bytes(subprocess.check_output(['git', 'diff', '--', 'crates/native/src/worker.rs'], cwd=checkout))
        with (output / (label + '.build.log')).open('w') as log:
            subprocess.run(['cargo', 'build', '--release', '--locked', '-p', 'core-scheduler-echo-benchmark'], cwd=checkout, stdout=log, stderr=subprocess.STDOUT, check=True)
        executable = output / label
        shutil.copy2(checkout / 'target/release/core-scheduler-echo-benchmark', executable)
        builds[label] = dict(base='b03e582', source_sha256=hashlib.sha256(source.encode()).hexdigest(), binary_sha256=hashlib.sha256(executable.read_bytes()).hexdigest())
        print('Built', label, builds[label]['binary_sha256'], flush=True)
    source_path.write_text(original)
    (output / 'build.json').write_text(json.dumps(builds, indent=2) + '\n')
