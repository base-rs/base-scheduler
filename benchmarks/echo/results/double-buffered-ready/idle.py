#!/usr/bin/env python3
"""Check that both worker loops park when echo servers have no clients."""

import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import time


def cpu_seconds(pid):
    # Fields after the parenthesized command start at stat field 3 (state).
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    return (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = {"measurement_seconds": 2, "warmup_seconds": 0.5, "binaries": {
        variant: {"path": str(binary), "sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
        for variant, binary in [("before", args.before), ("after", args.after)]
    }, "samples": []}
    with open("/tmp/core-scheduler-perf.lock", "a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        for workers in [1, 4]:
            for backend in ["tokio", "iouring", "iouring-poll"]:
                for variant, binary in [("before", args.before), ("after", args.after)]:
                    server = subprocess.Popen(
                        ["taskset", "-c", ",".join(map(str, range(workers))), str(binary),
                         "server", backend, str(workers), "1"],
                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                        start_new_session=True,
                    )
                    try:
                        assert server.stdout.readline().strip(), "server did not start"
                        time.sleep(0.5)
                        started = time.monotonic()
                        before = cpu_seconds(server.pid)
                        time.sleep(2)
                        used = cpu_seconds(server.pid) - before
                        assert server.poll() is None, "idle server exited"
                        sample = {"variant": variant, "backend": backend, "workers": workers,
                                  "cpu_seconds": used, "elapsed_seconds": time.monotonic() - started}
                        report["samples"].append(sample)
                        print(sample, flush=True)
                    finally:
                        try:
                            os.killpg(server.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                        server.wait()
                        server.stdout.close()
                        server.stderr.close()
    args.output.write_text(json.dumps(report, indent=2) + "\n")
