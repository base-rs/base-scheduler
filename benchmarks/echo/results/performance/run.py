#!/usr/bin/env python3
"""Compare two release builds on Linux using one fixed baseline client."""

import argparse
import hashlib
import itertools
import json
import os
from pathlib import Path
import queue
import random
import resource
import signal
import subprocess
import tempfile
import threading


def usage():
    value = resource.getrusage(resource.RUSAGE_CHILDREN)
    return {"user": value.ru_utime, "system": value.ru_stime}


def difference(before, after):
    return {key: after[key] - value for key, value in before.items()}


def sample(server_binary, client_binary, backend, workers, connections, size):
    with tempfile.TemporaryFile(mode="w+") as errors:
        server = subprocess.Popen(
            ["taskset", "-c", ",".join(map(str, range(workers))), str(server_binary),
             "server", backend, str(workers), str(connections)],
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
                ["taskset", "-c", "4-7", str(client_binary), "client", address,
                 "4", str(connections), str(size), "1", "3"],
                capture_output=True, text=True, check=True, timeout=60,
            )
            after_client = usage()
            if server.wait(timeout=30):
                raise RuntimeError("server failed")
            return dict(json.loads(client.stdout), backend=backend, workers=workers,
                        client_cpu=difference(before, after_client),
                        server_cpu=difference(after_client, usage()))
        except BaseException:
            errors.seek(0)
            print(errors.read(), flush=True)
            raise
        finally:
            try:
                os.killpg(server.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            server.wait()
            server.stdout.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--improved", type=Path, required=True)
    parser.add_argument("--source", required=True, help="candidate source commit")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repeats", type=int, default=5)
    args = parser.parse_args()
    binaries = {name: path.resolve() for name, path in
                [("baseline", args.baseline), ("improved", args.improved)]}
    report = dict(candidate_source=args.source,
                  baseline_source="5736301fded46c4e5f11d5387950ae2ee0807b94",
                  binaries={name: {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
                            for name, path in binaries.items()},
                  server_cpus="0 through workers-1", client_cpus="4-7", client_workers=4,
                  client_binary="baseline", seed=20261021, samples=[])
    cases = list(itertools.product(range(1, args.repeats + 1), [(1, 1), (1, 32), (4, 32)],
                                   [64, 4096], [("baseline", "iouring"), ("improved", "iouring"),
                                                ("baseline", "tokio")]))
    random.Random(report["seed"]).shuffle(cases)
    for repeat, (workers, connections), size, (variant, backend) in cases:
        result = sample(binaries[variant], binaries["baseline"], backend, workers, connections, size)
        report["samples"].append(dict(result, variant=variant, repeat=repeat))
        args.output.write_text(json.dumps(report, indent=2) + "\n")
        print(f"{len(report['samples'])}/{len(cases)} {variant} {backend} w{workers} c{connections} "
              f"b{size} r{repeat}: {result['requests_per_second']:,.0f} req/s", flush=True)


if __name__ == "__main__":
    main()
