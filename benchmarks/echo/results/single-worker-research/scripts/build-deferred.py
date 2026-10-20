#!/usr/bin/env python3
"""Revisit deferred task work after the executor self-wake fix; separate diagnostic."""
import fcntl, hashlib, json, shutil, subprocess
from pathlib import Path
root = Path.home()/'.cache/core-scheduler-single-worker-research'
checkout = Path.home()/'Developer/core-rs/core-scheduler-single-worker-research'
with open('/tmp/core-scheduler-perf.lock','a') as lock:
    fcntl.flock(lock,fcntl.LOCK_EX)
    source = checkout/'crates/iouring/src/io_uring_selector.rs'
    text = subprocess.check_output(['git','show','HEAD:crates/iouring/src/io_uring_selector.rs'],cwd=checkout,text=True)
    old = '.setup_coop_taskrun()'
    assert text.count(old) == 1
    source.write_text(text.replace(old,'.setup_single_issuer()\n                    .setup_defer_taskrun()'))
    with (root/'deferred-build.log').open('w') as output:
        subprocess.run(['cargo','build','--release','--locked','-p','core-scheduler-echo-benchmark'],cwd=checkout,stdout=output,stderr=subprocess.STDOUT,check=True)
    binary = root/'deferred'
    shutil.copy2(checkout/'target/release/core-scheduler-echo-benchmark',binary)
    patch = subprocess.check_output(['git','diff','--','crates'],cwd=checkout,text=True)
    (root/'deferred.patch').write_text(patch)
    (root/'deferred-build.json').write_text(json.dumps({
        'base_source':subprocess.check_output(['git','rev-parse','HEAD'],cwd=checkout,text=True).strip(),
        'production_patch_sha256':hashlib.sha256(patch.encode()).hexdigest(),
        'benchmark_patch':'controls.patch (unchanged in both variants)',
        'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),
        'baseline_binary_sha256':hashlib.sha256((root/'controls').read_bytes()).hexdigest(),
        'build_command':'cargo build --release --locked -p core-scheduler-echo-benchmark',
        'scope':'Only fresh worker-bound rings; original fallback and standalone rings unchanged. Separate from POLL_FIRST.',
    },indent=2)+'\n')
print('Built',binary,flush=True)
