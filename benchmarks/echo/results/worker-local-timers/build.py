"""Rebuild the two recorded runtime variants without resetting existing worktrees."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

repository = Path(__file__).resolve().parents[4]
home = Path.home()
cache = home / ".cache/core-scheduler-worker-timers-production"
cache.mkdir(parents=True, exist_ok=True)
report = {"base": "1689f8d6d7014a3dcd983b5e88036dc40aa4f527", "builds": {},
          "rustc": subprocess.check_output(["rustc", "-Vv"], text=True),
          "overrides": {k: v for k, v in os.environ.items() if k.startswith(("RUST", "CARGO_PROFILE"))}}
with open("/tmp/core-scheduler-perf.lock", "a") as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    for label, name, revision in [
        ("before", "baseline", report["base"]),
        ("after", "integrated", "eb0954c7afb566d92173124183f2a860846548fb"),
    ]:
        checkout = home / "Developer/core-rs" / ("core-scheduler-worker-timers-" + name)
        if not checkout.exists():
            subprocess.run(["git", "worktree", "add", "--detach", str(checkout), revision], cwd=repository, check=True)
        assert subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=checkout, text=True).strip() == revision
        assert not subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=checkout)
        unrelated = subprocess.check_output(["git", "ls-files", "--others", "--exclude-standard"], cwd=checkout, text=True).splitlines()
        assert all(path == "benchmarks/echo/src/bin/timers.rs" for path in unrelated), unrelated
        diagnostic = checkout / "benchmarks/echo/src/bin/timers.rs"
        diagnostic.parent.mkdir(exist_ok=True)
        shutil.copy2(Path(__file__).with_name("timers.rs"), diagnostic)
        command = ["cargo", "build", "--release", "--locked", "-p", "core-scheduler-echo-benchmark", "--bins"]
        env = dict(os.environ, CARGO_TARGET_DIR=str(cache / "target"))
        with (cache / (label + "-build.log")).open("w") as log:
            subprocess.run(command, cwd=checkout, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        binaries = {}
        for binary in ["timers", "core-scheduler-echo-benchmark"]:
            destination = cache / (label + "-" + binary)
            shutil.copy2(cache / "target/release" / binary, destination)
            binaries[binary] = {"path": str(destination), "sha256": hashlib.sha256(destination.read_bytes()).hexdigest()}
        report["builds"][label] = {"command": command, "revision": revision, "binaries": binaries}
    (cache / "build.json").write_text(json.dumps(report, indent=2) + "\n")
