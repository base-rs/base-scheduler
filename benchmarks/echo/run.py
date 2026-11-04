#!/usr/bin/env python3
"""Run independent echo server/client processes and preserve every sample."""

import argparse
import datetime
import hashlib
import itertools
import json
import os
from pathlib import Path
import platform
import random
import queue
import threading
import statistics
import subprocess
import sys
import tempfile
try:
    import resource
except ImportError:  # Windows
    resource = None
import time

ROOT = Path(__file__).resolve().parents[2]


def cpu_list(value):
    cpus = []
    for item in value.split(","):
        bounds = [int(part) for part in item.split("-")]
        cpus.extend(range(bounds[0], bounds[-1] + 1))
    return cpus


def affinity(cpus):
    return ["taskset", "--cpu-list", ",".join(map(str, cpus))] if cpus else []


def command_output(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def reap(server, timeout):
    """Wait for the server and return its exit status and resource usage, or None where the platform lacks wait4."""
    if not hasattr(os, "wait4"):
        return server.wait(timeout=timeout), None
    deadline = time.monotonic() + timeout
    while True:
        pid, status, usage = os.wait4(server.pid, os.WNOHANG)
        if pid:
            server.returncode = os.waitstatus_to_exitcode(status)
            return server.returncode, usage
        if time.monotonic() > deadline:
            raise subprocess.TimeoutExpired(server.args, timeout)
        time.sleep(0.01)


def sample(binary, backend, workers, connections, size, args, rate=None):
    server_cpus = args.server_cpus[:workers] if args.server_cpus else []
    server_command = affinity(server_cpus) + [str(binary), "server", backend, str(workers), str(connections)]
    client_prefix = affinity(args.client_cpus)
    with tempfile.TemporaryFile(mode="w+") as errors:
        server = subprocess.Popen(server_command, cwd=ROOT, stdout=subprocess.PIPE, stderr=errors, text=True)
        try:
            ready = queue.Queue()
            threading.Thread(target=lambda: ready.put(server.stdout.readline()), daemon=True).start()
            try:
                address = ready.get(timeout=30).strip()
            except queue.Empty:
                raise RuntimeError("server did not become ready within 30 seconds") from None
            if not address:
                raise RuntimeError("server exited before printing its address")
            client_command = client_prefix + [str(args.client_binary or binary), "client", address, str(args.client_workers), str(connections), str(size), str(args.warmup), str(args.seconds)] + ([str(rate)] if rate is not None else [])
            children = resource.getrusage(resource.RUSAGE_CHILDREN) if resource else None
            client = subprocess.run(client_command, cwd=ROOT, capture_output=True, text=True, timeout=args.warmup + args.seconds + 30, check=True)
            status, usage = reap(server, 30)
            if status:
                raise RuntimeError(f"server exited with status {status}")
            data = json.loads(client.stdout)
            result = dict(data, backend=backend, workers=workers, client_workers=args.client_workers, server_cpus=server_cpus, client_cpus=args.client_cpus)
            if usage:
                # Server and client CPU cover each process's whole life, including setup, warmup and shutdown. Both processes were reaped since `children` was read, so the client's share is the remainder.
                server_cpu = usage.ru_utime + usage.ru_stime
                after = resource.getrusage(resource.RUSAGE_CHILDREN)
                client_cpu = (after.ru_utime + after.ru_stime) - (children.ru_utime + children.ru_stime) - server_cpu
                result.update(server_cpu_seconds=server_cpu, server_user_seconds=usage.ru_utime, server_system_seconds=usage.ru_stime, client_cpu_seconds=round(client_cpu, 6))
            return result
        except Exception as error:
            if isinstance(error, subprocess.CalledProcessError):
                print(error.stderr, file=sys.stderr)
            errors.seek(0)
            print(errors.read(), file=sys.stderr)
            raise
        finally:
            if server.poll() is None:
                server.kill()
            server.wait()
            server.stdout.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workers", type=int, nargs="+", default=[1, 4])
    parser.add_argument("--connections", type=int, nargs="+", default=[1, 32])
    parser.add_argument("--bytes", type=int, nargs="+", default=[64, 4096])
    parser.add_argument("--backends", nargs="+", default=["tokio", "tokio-adapter", "tokio-adapter-poll", "iouring", "iouring-poll"])
    parser.add_argument("--client-binary", type=Path, help="reuse a fixed client executable instead of the newly built binary")
    parser.add_argument("--client-workers", type=int, default=4)
    parser.add_argument("--warmup", type=float, default=1)
    parser.add_argument("--seconds", type=float, default=3)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--rate", type=int, help="offered requests per second across all connections; unrestricted by default")
    parser.add_argument("--server-cpus", type=cpu_list, default=[])
    parser.add_argument("--client-cpus", type=cpu_list, default=[])
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.client_binary:
        args.client_binary = args.client_binary.resolve()
        if not args.client_binary.is_file():
            parser.error("client binary must exist")
    if min(args.workers + args.connections + args.bytes + [args.client_workers, args.repeats]) < 1 or args.warmup < 0 or args.seconds <= 0:
        parser.error("counts and measurement duration must be positive; warmup must be nonnegative")
    if args.rate is not None and args.rate < 1:
        parser.error("the offered rate must be positive")
    if args.rate is not None and sys.platform == "win32":
        parser.error("--rate needs timers finer than Windows' default 15.6 ms resolution")
    if args.rate is not None and args.client_cpus and len(args.client_cpus) <= args.client_workers:
        print("warning: paced client workers spin before each send; give the client more CPUs than --client-workers, or preemption adds milliseconds to latency tails", file=sys.stderr)
    if args.server_cpus and len(args.server_cpus) < max(args.workers):
        parser.error("provide at least one server CPU per worker")
    if set(args.server_cpus) & set(args.client_cpus):
        parser.error("server and client CPU sets must be disjoint")
    binary = ROOT / "target" / "release" / ("base-scheduler-echo-benchmark.exe" if sys.platform == "win32" else "base-scheduler-echo-benchmark")
    subprocess.run(["cargo", "build", "--release", "--locked", "-p", "base-scheduler-echo-benchmark"], cwd=ROOT, check=True)
    report = {
        "created_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "hostname": platform.node(), "platform": platform.platform(),
        "commit": command_output("git", "rev-parse", "HEAD"),
        "dirty": bool(command_output("git", "status", "--porcelain")),
        "rustc": command_output("rustc", "--version"),
        "cargo": command_output("cargo", "--version"),
        "arguments": dict(vars(args), output=str(args.output), client_binary=str(args.client_binary) if args.client_binary else None),
        "binaries": {name: {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
                     for name, path in [("server", binary), ("client", args.client_binary or binary)]},
        "cpu_topology": command_output("lscpu", "-e=CPU,CORE,SOCKET,NODE,CACHE") if sys.platform == "linux" else None,
        "cpu_model": command_output("lscpu") if sys.platform == "linux" else platform.processor(),
        "load_average_before": os.getloadavg() if hasattr(os, "getloadavg") else None,
        "samples": [],
    }
    rng = random.Random(20261019)
    for repeat in range(args.repeats):
        cases = list(itertools.product(args.workers, args.connections, args.bytes))
        rng.shuffle(cases)
        for workers, connections, size in cases:
            backends = args.backends.copy()
            rng.shuffle(backends)
            for backend in backends:
                data = sample(binary, backend, workers, connections, size, args, args.rate)
                report["samples"].append(dict(data, repeat=repeat + 1))
                args.output.parent.mkdir(parents=True, exist_ok=True)
                args.output.write_text(json.dumps(report, indent=2) + "\n")
                print(f"{len(report['samples']):3} {backend:13} workers={workers} conns={connections:3} bytes={size:5}: {data['requests_per_second']:10.0f} req/s  p50={data['rtt_p50_us']:8.2f} us p99={data['rtt_p99_us']:8.2f} us" + (f"  server cpu={data['server_cpu_seconds']:.3f} s" if "server_cpu_seconds" in data else "") + (f"  send delay p99={data['paced_send_delay_p99_us']:.1f} us" if data.get("paced_send_delay_p99_us") is not None else ""), flush=True)
    report["load_average_after"] = os.getloadavg() if hasattr(os, "getloadavg") else None
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print("\nMedian requests/s across repetitions:")
    for workers, connections, size in itertools.product(args.workers, args.connections, args.bytes):
        values = []
        for backend in args.backends:
            selected = [row["requests_per_second"] for row in report["samples"] if (row["workers"], row["connections"], row["bytes"], row["backend"]) == (workers, connections, size, backend)]
            values.append(f"{backend}={statistics.median(selected):.0f}")
        print(f"workers={workers} conns={connections} bytes={size}: " + "  ".join(values))


if __name__ == "__main__":
    main()
