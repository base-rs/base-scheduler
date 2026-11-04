#!/usr/bin/env python3
"""Run the stall rescue acceptance workloads with `diagnostics` builds of the timer and echo benchmarks, and record each run's rescue counts. Timing from these builds is not a measurement."""
import argparse, importlib.util, json, pathlib, random, subprocess, types

ROOT = pathlib.Path(__file__).resolve().parents[4]
spec = importlib.util.spec_from_file_location("echo_run", ROOT / "benchmarks/echo/run.py")
echo = importlib.util.module_from_spec(spec); spec.loader.exec_module(echo)

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--timers", required=True, help="base-scheduler-benchmark-timers built with --features diagnostics")
parser.add_argument("--echo", required=True, help="base-scheduler-benchmark-echo built with --features diagnostics")
parser.add_argument("--repeats", type=int, default=3)
parser.add_argument("--server-cpus", type=echo.cpu_list, default=[])
parser.add_argument("--client-cpus", type=echo.cpu_list, default=[])
parser.add_argument("--output", type=pathlib.Path, required=True)
args = parser.parse_args()

def timers(*arguments):
    return json.loads(subprocess.check_output([args.timers, *map(str, arguments)], timeout=300))

def echo_sample(backend, workers, connections, size, rate, warmup, seconds):
    options = types.SimpleNamespace(server_cpus=args.server_cpus, client_cpus=args.client_cpus, client_binary=args.echo, client_workers=4, warmup=warmup, seconds=seconds)
    return echo.sample(args.echo, backend, workers, connections, size, options, rate)

# The workloads of the stage 2 acceptance run (`benchmarks/echo/results/stall-rescue/scripts/budget.py`), plus `stall`, a control that must record rescues.
cases = []
for workload, workers, iterations in [("spawn", 4, 20000), ("spawn-pending", 4, 20000), ("root-sleep", 4, 1000), ("task-sleep", 4, 300), ("task-sleep-pending", 4, 300), ("idle", 4, 10000), ("pulse", 4, 10), ("stall", 2, 3)]:
    cases.append((f"scenario {workload}", lambda a=(workload, workers, iterations): timers("scenario", *a)))
for backend in ["native", "iouring"]:
    for mode, workload, tasks, iterations, workers in [("sleep", "refresh", 1, 3_000_000, 1), ("reuse", "refresh", 1, 3_000_000, 1), ("sleep", "yield", 64, 75_000, 4), ("reuse", "yield", 64, 75_000, 4), ("sleep", "expire", 128, 200, 4), ("reuse", "expire", 128, 200, 4), ("sleep", "cycle", 64, 200, 4), ("reuse", "cycle", 64, 200, 4)]:
        cases.append((f"timers {backend} {mode} {workload} {tasks}/{workers}", lambda a=(backend, mode, workload, tasks, iterations, workers): timers(*a, 1000)))
    for workers, tasks, delay, alignment in [(1, 1, 10, "unaligned"), (4, 1024, 1000, "unaligned"), (4, 1024, 1000, "aligned")]:
        cases.append((f"periodic {backend} {workers}/{tasks}/{delay}us {alignment}", lambda a=(backend, workers, tasks, delay, alignment): timers("periodic", *a)))
for backend in ["native", "native-poll", "iouring", "iouring-poll"]:
    for (workers, connections, size), rate in [((1, 1, 64), 35_000), ((1, 32, 64), 100_000), ((4, 128, 64), 220_000), ((4, 128, 4096), 190_000)]:
        cases.append((f"echo {backend} {workers}/{connections}/{size}", lambda a=(backend, workers, connections, size): echo_sample(*a, None, 1, 3)))
        cases.append((f"echo {backend} {workers}/{connections}/{size} @{rate}", lambda a=(backend, workers, connections, size, rate): echo_sample(*a, 1, 3)))
    cases.append((f"echo {backend} 1/1/64 @1 idle", lambda a=(backend,): echo_sample(a[0], 1, 1, 64, 1, 0, 10)))

rng = random.Random(20261105)
with args.output.open("a") as output:
    for repeat in range(args.repeats):
        order = list(range(len(cases))); rng.shuffle(order)
        for index in order:
            name, run = cases[index]
            output.write(json.dumps({"case": name, "repeat": repeat, "data": run()}) + "\n"); output.flush()
        print(f"repeat {repeat + 1}/{args.repeats} complete", flush=True)
