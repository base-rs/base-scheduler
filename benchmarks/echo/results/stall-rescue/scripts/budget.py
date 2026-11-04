#!/usr/bin/env python3
"""Paired, randomly ordered runs of the `main` and stall-rescue builds for the stage 2 budgets. Run under the performance lock."""
import importlib.util, json, os, random, subprocess, sys, types

ROOT = os.environ.get("BASE_SCHEDULER", os.path.abspath(os.path.join(os.path.dirname(__file__), "../../../../..")))
BENCH = os.environ.get("STALL_RESCUE_BENCH", "/tmp/bs-bench")
spec = importlib.util.spec_from_file_location("echo_run", f"{ROOT}/benchmarks/echo/run.py")
echo = importlib.util.module_from_spec(spec); spec.loader.exec_module(echo)
BUILDS = ["main", "branch"]

def rb(build, workload, iterations):
    return json.loads(subprocess.check_output([f"{BENCH}/rb-{build}/target/release/rb", workload, "4", str(iterations)], timeout=120))

def timers(build, backend, mode, workload, tasks, iterations, workers):
    return json.loads(subprocess.check_output([f"{BENCH}/timers-{build}", backend, mode, workload, str(tasks), str(iterations), str(workers), "1000"], timeout=120))

def diag(build, backend, workers, tasks, delay, aligned):
    env = dict(os.environ)
    if aligned:
        env["CORE_TIMER_ALIGNED"] = "1"
    return json.loads(subprocess.check_output([f"{BENCH}/diag-{build}/target/release/diag", backend, str(workers), str(tasks), str(delay)], env=env, timeout=120))

def echo_sample(build, backend, workers, connections, size, rate, warmup, seconds):
    args = types.SimpleNamespace(server_cpus=[0, 4, 1, 5], client_cpus=[2, 6, 3, 7], client_binary=f"{BENCH}/echo-branch", client_workers=4, warmup=warmup, seconds=seconds)
    return echo.sample(f"{BENCH}/echo-{build}", backend, workers, connections, size, args, rate)

cases = []
for workload, iterations in [("spawn", 20000), ("spawn-pending", 20000), ("root-sleep", 1000), ("task-sleep", 300), ("task-sleep-pending", 300), ("idle", 10000), ("pulse", 10)]:
    cases.append((f"rb {workload}", lambda b, w=workload, i=iterations: rb(b, w, i)))
for backend in ["native", "iouring"]:
    for mode, workload, tasks, iterations, workers in [("sleep", "refresh", 1, 3_000_000, 1), ("reuse", "refresh", 1, 3_000_000, 1), ("sleep", "yield", 64, 75_000, 4), ("reuse", "yield", 64, 75_000, 4), ("sleep", "expire", 128, 200, 4), ("reuse", "expire", 128, 200, 4), ("sleep", "cycle", 64, 200, 4), ("reuse", "cycle", 64, 200, 4)]:
        cases.append((f"timers {backend} {mode} {workload} {tasks}/{workers}", lambda b, a=(backend, mode, workload, tasks, iterations, workers): timers(b, *a)))
    for workers, tasks, delay, aligned in [(1, 1, 10, False), (4, 1024, 1000, False), (4, 1024, 1000, True)]:
        cases.append((f"diag {backend} {workers}/{tasks}/{delay}us{' aligned' if aligned else ''}", lambda b, a=(backend, workers, tasks, delay, aligned): diag(b, *a)))
for backend in ["native", "native-poll", "iouring", "iouring-poll"]:
    for (workers, connections, size), rate in [((1, 1, 64), 35_000), ((1, 32, 64), 100_000), ((4, 128, 64), 220_000), ((4, 128, 4096), 190_000)]:
        cases.append((f"echo {backend} {workers}/{connections}/{size}", lambda b, a=(backend, workers, connections, size): echo_sample(b, *a, None, 1, 3)))
        cases.append((f"echo {backend} {workers}/{connections}/{size} @{rate}", lambda b, a=(backend, workers, connections, size, rate): echo_sample(b, *a, 1, 3)))
    cases.append((f"echo {backend} 1/1/64 @1 idle", lambda b, a=(backend,): echo_sample(b, a[0], 1, 1, 64, 1, 0, 10)))

repeats = int(sys.argv[1]); output = sys.argv[2]
rng = random.Random(20261103)
with open(output, "a") as out:
    for repeat in range(repeats):
        order = list(range(len(cases))); rng.shuffle(order)
        for index in order:
            name, run = cases[index]
            builds = BUILDS.copy(); rng.shuffle(builds)
            for build in builds:
                data = run(build)
                out.write(json.dumps({"case": name, "build": build, "repeat": repeat, "data": data}) + "\n"); out.flush()
        print(f"repeat {repeat + 1}/{repeats} complete", flush=True)
