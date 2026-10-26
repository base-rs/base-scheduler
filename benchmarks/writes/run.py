#!/usr/bin/env python3
"""Run serialized, randomized write comparisons on Linux and retain every sample."""

import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import random
import selectors
import subprocess
import time


BACKENDS = ["tokio", "tokio-monitor", "tokio-writer", "iouring-monitor", "iouring-writer"]

# workers, connections, bytes, chunks, work per cache line, requested SO_SNDBUF
CASES = {
    "screen": [
        (1, 1, 64, 1, 0, 0),
        (1, 32, 4096, 1, 0, 0),
        (1, 1, 65536, 8, 8, 0),
        (1, 1, 262144, 8, 8, 65536),
        (4, 32, 262144, 8, 8, 65536),
    ],
    "immediate": [
        (workers, connections, size, 1, 0, 0)
        for workers in (1, 4)
        for connections in (1, 32)
        for size in (64, 4096)
    ],
    "overlap": [
        (workers, connections, size, 8, work, send_buffer)
        for workers, connections in ((1, 1), (1, 32), (4, 32))
        for work in (0, 8)
        for size, send_buffer in ((65536, 0), (262144, 65536))
    ],
}


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def command(*args):
    return subprocess.check_output(args, text=True).strip()


def save(path, value):
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    temporary.replace(path)


def sample(server_binary, client_binary, backend, case, warmup, seconds, stat_path=None):
    workers, connections, size, chunks, work, send_buffer = case
    affinity = "0" if workers == 1 else f"0-{workers - 1}"
    server_command = ["taskset", "-c", affinity, server_binary, "server", backend,
                      str(workers), str(connections), str(size), str(chunks), str(work), str(send_buffer)]
    if stat_path:
        server_command = ["sudo", "-n", "perf", "stat", "-x,", "-o", str(stat_path),
                          "-e", "cycles,instructions,cache-misses,syscalls:sys_enter_io_uring_enter,syscalls:sys_enter_sendto",
                          "-e", "io_uring:io_uring_submit_req", "--filter", "opcode == 26", "--"] + server_command
    server = subprocess.Popen(server_command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        with selectors.DefaultSelector() as ready:
            ready.register(server.stdout, selectors.EVENT_READ)
            if not ready.select(timeout=30):
                raise TimeoutError("server did not announce its address")
        address = server.stdout.readline().strip()
        if not address:
            raise RuntimeError(f"server startup failed: {server.stderr.read()}")
        client_command = ["taskset", "-c", "4-7", client_binary, "client", address,
                          "4", str(connections), str(size), str(chunks), str(work), str(warmup), str(seconds)]
        client = subprocess.run(client_command, text=True, capture_output=True, timeout=warmup + seconds + 60)
        if client.returncode:
            raise RuntimeError(f"client failed: {client.stderr}")
        remaining_stdout, server_stderr = server.communicate(timeout=30)
        if server.returncode:
            raise RuntimeError(f"server failed: {server_stderr}")
        return {
            "backend": backend,
            "workers": workers,
            "server_cpus": affinity,
            "client_cpus": "4-7",
            "client_workers": 4,
            "send_buffer": send_buffer,
            "server_command": server_command,
            "client_command": client_command,
            "server_stderr": server_stderr,
            "server_remaining_stdout": remaining_stdout,
            "perf_stat": str(stat_path) if stat_path else None,
            "result": json.loads(client.stdout),
        }
    finally:
        if server.poll() is None:
            server.kill()
            server.communicate()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/release/base-scheduler-write-benchmark")
    parser.add_argument("--client-binary")
    parser.add_argument("--variant", action="append", default=[], help="LABEL=BACKEND=SERVER_BINARY; use LABEL in --backends to compare isolated implementations")
    parser.add_argument("--perf-stat", type=Path, help="Diagnostic counter directory; instrumented throughput is not a capacity measurement")
    parser.add_argument("--backends", nargs="+", default=BACKENDS)
    parser.add_argument("--suite", choices=CASES, default="screen")
    parser.add_argument("--case", action="append", help="Override suite with workers,connections,bytes,chunks,work,send_buffer; repeatable")
    parser.add_argument("--repetitions", type=int, default=5)
    parser.add_argument("--warmup", type=float, default=1)
    parser.add_argument("--seconds", type=float, default=3)
    parser.add_argument("--seed", type=int, default=20261024)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.repetitions <= 0 or args.warmup < 0 or args.seconds <= 0:
        parser.error("positive repetitions and seconds, nonnegative warmup required")
    cases = [tuple(map(int, item.split(","))) for item in args.case] if args.case else CASES[args.suite]
    if any(len(case) != 6 or case[0] not in (1, 4) for case in cases):
        parser.error("each case has six integers and 1 or 4 workers")
    binary = str(Path(args.binary).resolve())
    client_binary = str(Path(args.client_binary or args.binary).resolve())
    variants = {}
    for item in args.variant:
        label, backend, path = item.split("=", 2)
        variants[label] = {"backend": backend, "binary": str(Path(path).resolve()), "binary_sha256": digest(path)}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    if args.perf_stat:
        args.perf_stat = args.perf_stat.resolve()
        args.perf_stat.mkdir(parents=True, exist_ok=True)
    with open("/tmp/base-scheduler-perf.lock", "a") as lock:
        print("Waiting for /tmp/base-scheduler-perf.lock", flush=True)
        fcntl.flock(lock, fcntl.LOCK_EX)
        metadata = {
            "started_unix": time.time(),
            "hostname": command("hostname"),
            "uname": command("uname", "-a"),
            "rustc": command("rustc", "-vV"),
            "git_head": command("git", "rev-parse", "HEAD"),
            "git_status": command("git", "status", "--short"),
            "git_diff_sha256": hashlib.sha256(subprocess.check_output(["git", "diff", "HEAD"])).hexdigest(),
            "binary": binary,
            "binary_sha256": digest(binary),
            "client_binary": client_binary,
            "client_binary_sha256": digest(client_binary),
            "variants": variants,
            "instrumented": bool(args.perf_stat),
            "io_uring_submit_filter": "opcode == 26 (IORING_OP_SEND)" if args.perf_stat else None,
            "cargo_lock_sha256": digest("Cargo.lock"),
            "cpu": json.loads(command("lscpu", "-J")),
            "governors": {str(cpu): Path(f"/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor").read_text().strip() for cpu in range(8)},
            "suite": args.suite,
            "cases": cases,
            "repetitions": args.repetitions,
            "warmup": args.warmup,
            "seconds": args.seconds,
            "seed": args.seed,
            "backends": args.backends,
            "payload_validation": "Every response byte checked against precomputed chunk-specific contents.",
            "buffer_budget": "Two fixed payload buffers per connection for every server backend.",
        }
        report = {"metadata": metadata, "samples": []}
        schedule = [(repetition, backend, case) for repetition in range(args.repetitions) for case in cases for backend in args.backends]
        random.Random(args.seed).shuffle(schedule)
        save(args.output, report)
        for index, (repetition, backend, case) in enumerate(schedule):
            print(f"{index + 1}/{len(schedule)}: {backend} {case} repetition={repetition}", flush=True)
            record = {"repetition": repetition, "started_unix": time.time()}
            try:
                variant = variants.get(backend, {"backend": backend, "binary": binary})
                stat_path = args.perf_stat / f"{index:03}-{backend}-r{repetition}.csv" if args.perf_stat else None
                record.update(sample(variant["binary"], client_binary, variant["backend"], case, args.warmup, args.seconds, stat_path))
                record["server_backend"] = variant["backend"]
                record["backend"] = backend
            except Exception as error:
                record.update({"backend": backend, "case": case, "error": repr(error)})
                report["samples"].append(record)
                save(args.output, report)
                raise
            report["samples"].append(record)
            save(args.output, report)
        metadata["finished_unix"] = time.time()
        save(args.output, report)


if __name__ == "__main__":
    main()
