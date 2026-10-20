#!/usr/bin/env python3
"""Screen the kernel receive poll-first hint; diagnostic, no compatibility fallback."""
import fcntl
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

root = Path.home() / '.cache/core-scheduler-single-worker-research'
checkout = Path.home() / 'Developer/core-rs/core-scheduler-single-worker-research'
source = checkout / 'crates/iouring/src/io_uring_selector.rs'
text = source.read_text()
old = 'opcode::Recv::new(descriptor, pointer, length).build()'
assert text.count(old) == 1
source.write_text(text.replace(old, '''opcode::Recv::new(descriptor, pointer, length)
                        // Diagnostic: IORING_RECVSEND_POLL_FIRST from linux/io_uring.h.
                        .ioprio(u16::from(length != 0))
                        .build()'''))
with open('/tmp/core-scheduler-perf.lock', 'a') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    with (root / 'poll-first-build.log').open('w') as output:
        subprocess.run(['cargo', 'build', '--release', '--locked', '-p', 'core-scheduler-echo-benchmark'], cwd=checkout, stdout=output, stderr=subprocess.STDOUT, check=True)
    binary = root / 'poll-first'
    shutil.copy2(checkout / 'target/release/core-scheduler-echo-benchmark', binary)
patch = subprocess.check_output(['git', 'diff', '--', 'crates'], cwd=checkout, text=True)
(root / 'poll-first.patch').write_text(patch)
(root / 'poll-first-build.json').write_text(json.dumps({
    'base_source': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=checkout, text=True).strip(),
    'production_patch_sha256': hashlib.sha256(patch.encode()).hexdigest(),
    'benchmark_patch': 'controls.patch (unchanged in both variants)',
    'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
    'baseline_binary_sha256': hashlib.sha256((root/'controls').read_bytes()).hexdigest(),
    'build_command': 'cargo build --release --locked -p core-scheduler-echo-benchmark',
    'limitation': 'Only a Linux 7.2 diagnostic. POLL_FIRST availability and fallback on older supported kernels are not implemented. Zero-length Recv retains the original behavior.',
}, indent=2) + '\n')
print('Built', binary, flush=True)
