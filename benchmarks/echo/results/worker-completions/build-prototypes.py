#!/usr/bin/env python3
"""Build isolated diagnostic ring-policy variants from the recorded baseline."""

import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[4]
BASE = "29d47375bf9a1b0239e9229335beb4d8beb0825b"
EXPERIMENT = Path.home() / ".cache/core-scheduler-single-worker-completions"
SOURCE = ROOT / "crates/iouring/src/io_uring_selector.rs"


def replace_once(source, old, new):
    assert source.count(old) == 1, f"expected one replacement: {old[:80]!r}"
    return source.replace(old, new)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--variants", nargs="+", default=["deferred", "deferred-no-iowait", "coop-no-iowait"], choices=["deferred", "deferred-no-iowait", "coop-no-iowait", "deferred-collect", "coop-collect", "deferred-batch", "deferred-batch-collect", "single-issuer", "deferred-probe-collect", "coop-probe-collect"])
    args = parser.parse_args()
    original = subprocess.check_output(["git", "show", BASE + ":crates/iouring/src/io_uring_selector.rs"], cwd=ROOT, text=True)
    environment = os.environ | {"CARGO_PROFILE_RELEASE_DEBUG": "1", "CARGO_TARGET_DIR": str(EXPERIMENT / "target")}
    variants = {}
    with open("/tmp/core-scheduler-perf.lock", "a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        assert SOURCE.read_text() == original, "prototype source must start clean"
        try:
            for name in args.variants:
                source = original
                if name == "single-issuer":
                    source = replace_once(source, ".setup_coop_taskrun()", ".setup_single_issuer()\n                    .setup_coop_taskrun()")
                if name.startswith("deferred"):
                    source = replace_once(source, ".setup_coop_taskrun()", ".setup_single_issuer()\n                    .setup_defer_taskrun()")
                    source = replace_once(source, 'let result = syscall("submit", || self.ring.submit());', '''let result = syscall("submit", || {
                    let count = self.ring.submission().len() as u32;
                    // SAFETY: This diagnostic retains the exclusive ring owner and all submitted resources. Zero-minimum GETEVENTS does not block.
                    unsafe { self.ring.submitter().enter::<libc::sigset_t>(count, 0, io_uring::EnterFlags::GETEVENTS.bits(), None) }
                });''')
                if "-batch" in name:
                    source = replace_once(source, '''let count = self.ring.submission().len() as u32;
                    // SAFETY: This diagnostic retains the exclusive ring owner and all submitted resources. Zero-minimum GETEVENTS does not block.
                    unsafe { self.ring.submitter().enter::<libc::sigset_t>(count, 0, io_uring::EnterFlags::GETEVENTS.bits(), None) }''', '''let timeout = types::Timespec::new();
                    let args = types::SubmitArgs::new().timespec(&timeout);
                    // The zero deadline keeps this diagnostic nonblocking while asking for a complete available completion batch.
                    match self.ring.submitter().submit_with_args(self.ring.params().cq_entries() as usize, &args) {
                        Err(error) if error.raw_os_error() == Some(libc::ETIME) => Ok(0),
                        result => result,
                    }''')
                if name.endswith("no-iowait"):
                    source = replace_once(source, "None => self.ring.submit_and_wait(1),", '''None => {
                let count = self.ring.submission().len() as u32;
                let mut flags = io_uring::EnterFlags::GETEVENTS;
                if self.ring.params().is_feature_no_iowait() {
                    flags |= io_uring::EnterFlags::NO_IOWAIT;
                }
                // SAFETY: This diagnostic preserves the existing indefinite wait and retained SQE resources, while selecting the kernel-supported wait-accounting flag.
                unsafe { self.ring.submitter().enter::<libc::sigset_t>(count, 1, flags.bits(), None) }
            },''')
                if name.endswith("-collect"):
                    source = replace_once(source, '''            return Ok(false);
        }
        let result = syscall("wait", || match timeout {''', '''            self.collect_completions()?;
            return Ok(false);
        }
        let result = syscall("wait", || match timeout {''')
                if "-probe-" in name:
                    source = replace_once(source, "|| submissions.taskrun()", "|| submissions.taskrun()\n                    || submissions.is_empty()")
                assert source != original
                SOURCE.write_text(source)
                subprocess.run(["cargo", "fmt", "--all"], cwd=ROOT, check=True)
                patch = subprocess.check_output(["git", "diff", "--", str(SOURCE)], cwd=ROOT)
                (EXPERIMENT / (name + ".patch")).write_bytes(patch)
                with (EXPERIMENT / "logs" / (name + "-build.log")).open("w") as log:
                    subprocess.run(["cargo", "build", "--release", "--locked", "-p", "core-scheduler-echo-benchmark"], cwd=ROOT, env=environment, stdout=log, stderr=subprocess.STDOUT, check=True)
                binary = EXPERIMENT / "bin" / name
                shutil.copy2(EXPERIMENT / "target/release/core-scheduler-echo-benchmark", binary)
                variants[name] = {"sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "patch_sha256": hashlib.sha256(patch).hexdigest()}
                print(name, variants[name], flush=True)
        finally:
            SOURCE.write_text(original)
    (EXPERIMENT / ("prototype-builds-" + "-".join(args.variants) + ".json")).write_text(json.dumps(variants, indent=2) + "\n")


if __name__ == "__main__":
    main()
