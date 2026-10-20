#!/usr/bin/env python3
"""Test whether the sleep/driver transition explains serial overhead."""
import fcntl
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

checkout, output = map(Path, sys.argv[1:])
output.mkdir(parents=True, exist_ok=True)
path = checkout / 'crates/native/src/worker.rs'
original = subprocess.check_output(['git', 'show', 'b03e582:crates/native/src/worker.rs'], cwd=checkout, text=True)
sleep = '            Steal::Empty => {\n                worker.sleeping.store(true, Ordering::SeqCst);'
wake = '''            self.thread
                .get()
                .expect("sleeping worker has registered its thread")
                .unpark();'''
skip_wake = '''            let target = self.thread.get().expect("sleeping worker has registered its thread");
            if target.id() == thread::current().id() {
                return true;
            }
            target.unpark();'''
assert original.count(sleep) == original.count(wake) == 1
variants = {
    'pre-idle-probe': original.replace(sleep, '            Steal::Empty => {\n                drive(&mut driver, Some(Duration::ZERO));\n                worker.sleeping.store(true, Ordering::SeqCst);'),
    'skip-self-wake': original.replace(wake, skip_wake),
}
builds = {}
with open('/tmp/core-scheduler-perf.lock', 'a') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    for label, source in variants.items():
        path.write_text(source)
        (output / (label + '.patch')).write_bytes(subprocess.check_output(['git', 'diff', '--', 'crates/native/src/worker.rs'], cwd=checkout))
        with (output / (label + '.build.log')).open('w') as log:
            subprocess.run(['cargo', 'build', '--release', '--locked', '-p', 'core-scheduler-echo-benchmark'], cwd=checkout, stdout=log, stderr=subprocess.STDOUT, check=True)
        binary = output / label
        shutil.copy2(checkout / 'target/release/core-scheduler-echo-benchmark', binary)
        builds[label] = dict(base='b03e582', source_sha256=hashlib.sha256(source.encode()).hexdigest(), binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest())
        print('Built', label, flush=True)
    path.write_text(original)
    (output / 'build.json').write_text(json.dumps(builds, indent=2) + '\n')
