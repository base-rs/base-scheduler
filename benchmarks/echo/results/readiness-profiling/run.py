#!/usr/bin/env python3
"""Profile current native readiness, owned completions, and pure Tokio on Linux."""

import argparse
import hashlib
import itertools
import json
import os
from pathlib import Path
import platform
import queue
import random
import resource
import signal
import subprocess
import sys
import threading
import time


def usage():
    value = resource.getrusage(resource.RUSAGE_CHILDREN)
    return dict(user=value.ru_utime, system=value.ru_stime,
                voluntary_switches=value.ru_nvcsw, involuntary_switches=value.ru_nivcsw)


def difference(before, after):
    return {key: after[key] - value for key, value in before.items()}


def sample(args, mode, backend, workers, connections, size, repeat):
    identifier = f"{mode}-{backend}-w{workers}-c{connections}-b{size}-r{repeat}"
    prefix = []
    if mode == "perf":
        prefix = ["perf", "record", "-q", "-m", "32", "-e", args.perf_event, "-F", "999",
                  "--call-graph", "dwarf,8192", "-o", str(args.output / (identifier + ".data")), "--"]
    elif mode == "stat":
        events = ["task-clock", "cycles", "instructions", "context-switches", "cpu-migrations"]
        events += ["syscalls:sys_enter_" + name for name in [
            "recvfrom", "sendto", "io_uring_enter", "epoll_ctl", "epoll_wait", "epoll_pwait", "read", "write", "futex",
        ]]
        prefix = ["perf", "stat", "-x", ";", "-e", ",".join(events),
                  "-o", str(args.output / (identifier + ".stat")), "--"]
    elif mode == "strace":
        prefix = ["strace", "-f", "-c", "-S", "calls", "-o", str(args.output / (identifier + ".strace"))]
    privileged = args.privileged_perf and mode in ["perf", "stat"]
    if privileged:
        # Give only the profiler elevated access; execute the server as the original user.
        # setpriv execs in place, keeping descendants in the supervisor's group.
        prefix = ["sudo", "-n", sys.executable, str(Path(__file__).with_name("supervise.py"))] + prefix + [
            "setpriv", f"--reuid={os.getuid()}", f"--regid={os.getgid()}", "--init-groups", "--",
        ]
    warmup, seconds = (0, 0.5) if mode == "strace" else ((0, 3) if mode == "stat" else (1, 5 if mode == "perf" else 3))
    with (args.output / (identifier + ".stderr")).open("w+") as errors:
        started = time.monotonic()
        server = subprocess.Popen(
            prefix + ["taskset", "-c", ",".join(map(str, range(workers))), str(args.server),
                      "server", backend, str(workers), str(connections)],
            stdin=subprocess.PIPE if privileged else subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=errors, text=True, start_new_session=True,
        )
        try:
            ready = queue.Queue()
            threading.Thread(target=lambda: ready.put(server.stdout.readline()), daemon=True).start()
            address = ready.get(timeout=30).strip()
            if not address:
                raise RuntimeError("server exited before printing its address")
            before = usage()
            client = subprocess.run(
                ["taskset", "-c", "4-7", str(args.client), "client", address, "4",
                 str(connections), str(size), str(warmup), str(seconds)],
                capture_output=True, text=True, check=True, timeout=warmup + seconds + 60,
            )
            after_client = usage()
            if server.wait(timeout=30):
                raise RuntimeError("server or profiler failed")
            return dict(json.loads(client.stdout), identifier=identifier, mode=mode,
                        backend=backend, workers=workers, repeat=repeat,
                        client_cpu=difference(before, after_client),
                        server_cpu=difference(after_client, usage()),
                        lifecycle_seconds=time.monotonic() - started)
        except BaseException:
            errors.seek(0)
            print(errors.read(), flush=True)
            raise
        finally:
            if privileged:
                # The privileged supervisor owns termination of its process group.
                server.stdin.close()
                server.wait(timeout=10)
            else:
                # A profiler can exit while its server child still owns stdout.
                try:
                    os.killpg(server.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                server.wait()
            server.stdout.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, required=True)
    parser.add_argument("--client", type=Path, required=True)
    parser.add_argument("--source", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--modes", nargs="+", choices=["control", "perf", "strace", "stat"],
                        default=["control", "perf", "strace"])
    parser.add_argument("--perf-event", default="cycles:u")
    parser.add_argument("--privileged-perf", action="store_true")
    args = parser.parse_args()
    if args.repeats < 1:
        parser.error("repeats must be positive")
    args.server, args.client, args.output = (path.resolve() for path in [args.server, args.client, args.output])
    # Keep earlier evidence intact; each invocation gets a new result directory.
    args.output.mkdir(parents=True, exist_ok=False)
    report = dict(
        source=args.source, client_source="5736301fded46c4e5f11d5387950ae2ee0807b94",
        binaries={name: dict(path=str(path), sha256=hashlib.sha256(path.read_bytes()).hexdigest())
                  for name, path in [("server", args.server), ("client", args.client)]},
        platform=platform.platform(), hostname=platform.node(),
        rustc=subprocess.check_output(["rustc", "--version"], text=True).strip(),
        perf_version=subprocess.check_output(["perf", "--version"], text=True).strip(),
        strace_version=subprocess.check_output(["strace", "--version"], text=True).splitlines()[0],
        perf_event_paranoid=Path("/proc/sys/kernel/perf_event_paranoid").read_text().strip(),
        cpu_topology=subprocess.check_output(["lscpu", "-e=CPU,CORE,SOCKET,NODE,CACHE"], text=True),
        server_cpus="0 through workers-1", client_cpus="4-7", client_workers=4,
        seed=20261023, repeats=args.repeats, modes=args.modes,
        perf_event=args.perf_event, privileged_perf=args.privileged_perf,
        load_average_before=os.getloadavg(), samples=[],
    )
    # Profile and trace separately from controls. Neither instrumented throughput nor CPU totals are capacity measurements.
    for mode in args.modes:
        repeats = 1 if mode == "strace" else args.repeats
        cases = list(itertools.product(
            range(1, repeats + 1), [(1, 1, 64), (1, 1, 4096), (1, 32, 64), (4, 32, 64)],
            ["tokio", "iouring", "iouring-poll"],
        ))
        random.Random(report["seed"] + ["control", "perf", "strace", "stat"].index(mode)).shuffle(cases)
        for repeat, (workers, connections, size), backend in cases:
            result = sample(args, mode, backend, workers, connections, size, repeat)
            report["samples"].append(result)
            (args.output / "aiko.json").write_text(json.dumps(report, indent=2) + "\n")
            print(f"{result['identifier']}: {result['requests_per_second']:,.0f} req/s", flush=True)
    report["load_average_after"] = os.getloadavg()
    (args.output / "aiko.json").write_text(json.dumps(report, indent=2) + "\n")
    # Report generation happens only after all timed workloads finish.
    for result in report["samples"]:
        if result["mode"] != "perf":
            continue
        identifier = result["identifier"]
        data = args.output / (identifier + ".data")
        perf = ["sudo", "-n", "perf"] if args.privileged_perf else ["perf"]
        with (args.output / (identifier + ".self.txt")).open("w") as output:
            subprocess.run(perf + ["report", "--stdio", "--header", "--no-children", "-g", "none",
                            "--show-nr-samples", "--sort", "symbol", "--percent-limit", "0.1",
                            "-i", str(data)], stdout=output, check=True)
        if result["repeat"] == 1:
            with (args.output / (identifier + ".callgraph.txt")).open("w") as output:
                subprocess.run(perf + ["report", "--stdio", "--children", "-g", "graph,1,caller",
                                "--sort", "symbol", "--percent-limit", "2", "-i", str(data)],
                               stdout=output, check=True)


if __name__ == "__main__":
    main()
