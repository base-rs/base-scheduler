#!/usr/bin/env python3
"""A few-minute paired comparison of `main` and a candidate: nominal costs and stalled-worker latency. Metrics are lower-is-better; requests per second are negated."""
import importlib.util, json, os, random, statistics, subprocess, sys, types

B = os.environ.get("STALL_RESCUE_BENCH", os.path.dirname(os.path.abspath(__file__)))
spec = importlib.util.spec_from_file_location("echo_run", f"{B}/branch/benchmarks/echo/run.py")
echo = importlib.util.module_from_spec(spec); spec.loader.exec_module(echo)

def out(cmd, env=None):
    return json.loads(subprocess.check_output(cmd, env=env, timeout=120))
def echo_sample(build, backend, w, c, s, rate, warmup, seconds):
    args = types.SimpleNamespace(server_cpus=[0, 4, 1, 5], client_cpus=[2, 6, 3, 7], client_binary=f"{B}/echo-branch", client_workers=4, warmup=warmup, seconds=seconds)
    return echo.sample(f"{B}/echo-{build}", backend, w, c, s, args, rate)

# name: (run(build) -> data, {metric: extractor}); lower is better for every metric.
CASES = {
    "refresh 1/1": (lambda b: out([f"{B}/timers-{b}", "native", "sleep", "refresh", "1", "1000000", "1", "1000"]), {"ns/op": lambda d: d["ns_per_operation"]}),
    "yield 64/4": (lambda b: out([f"{B}/timers-{b}", "native", "reuse", "yield", "64", "25000", "4", "1000"]), {"ns/op": lambda d: d["ns_per_operation"]}),
    "cycle 64/4": (lambda b: out([f"{B}/timers-{b}", "native", "reuse", "cycle", "64", "800", "4", "1000"]), {"cpu ns/op": lambda d: d["cpu_ns_per_operation"], "late p50 us": lambda d: d["overshoot_p50_ns"] / 1e3}),
    "cycle sleep 64/4": (lambda b: out([f"{B}/timers-{b}", "native", "sleep", "cycle", "64", "800", "4", "1000"]), {"cpu ns/op": lambda d: d["cpu_ns_per_operation"], "late p50 us": lambda d: d["overshoot_p50_ns"] / 1e3, "late p99 us": lambda d: d["overshoot_p99_ns"] / 1e3}),
    "cycle iouring 64/4": (lambda b: out([f"{B}/timers-{b}", "iouring", "reuse", "cycle", "64", "800", "4", "1000"]), {"cpu ns/op": lambda d: d["cpu_ns_per_operation"], "late p50 us": lambda d: d["overshoot_p50_ns"] / 1e3, "late p99 us": lambda d: d["overshoot_p99_ns"] / 1e3}),
    "diag iouring 4/1024/1ms": (lambda b: out([f"{B}/diag-{b}/target/release/diag", "iouring", "4", "1024", "1000"]), {"cpu s": lambda d: d["cpu_seconds"], "late p50 us": lambda d: d["lateness_p50_us"], "late p99 us": lambda d: d["lateness_p99_us"]}),
    "diag 4/1024/1ms": (lambda b: out([f"{B}/diag-{b}/target/release/diag", "native", "4", "1024", "1000"]), {"cpu s": lambda d: d["cpu_seconds"], "late p50 us": lambda d: d["lateness_p50_us"], "late p99 us": lambda d: d["lateness_p99_us"]}),
    "echo native 1/1/64": (lambda b: echo_sample(b, "native", 1, 1, 64, None, 1, 3), {"req/s": lambda d: -d["requests_per_second"], "server cpu us/req": lambda d: d["server_cpu_seconds"] * 1e6 / d["requests"], "p99 us": lambda d: d["rtt_p99_us"]}),
    "echo native 1/32/64": (lambda b: echo_sample(b, "native", 1, 32, 64, None, 1, 3), {"req/s": lambda d: -d["requests_per_second"], "server cpu us/req": lambda d: d["server_cpu_seconds"] * 1e6 / d["requests"], "p99 us": lambda d: d["rtt_p99_us"]}),
    "echo native 4/128/64": (lambda b: echo_sample(b, "native", 4, 128, 64, None, 1, 3), {"req/s": lambda d: -d["requests_per_second"], "server cpu us/req": lambda d: d["server_cpu_seconds"] * 1e6 / d["requests"], "p99 us": lambda d: d["rtt_p99_us"]}),
    "echo native 4/128/4096": (lambda b: echo_sample(b, "native", 4, 128, 4096, None, 1, 3), {"req/s": lambda d: -d["requests_per_second"], "server cpu us/req": lambda d: d["server_cpu_seconds"] * 1e6 / d["requests"], "p99 us": lambda d: d["rtt_p99_us"]}),
    "echo native-poll 1/1/64": (lambda b: echo_sample(b, "native-poll", 1, 1, 64, None, 1, 3), {"req/s": lambda d: -d["requests_per_second"], "server cpu us/req": lambda d: d["server_cpu_seconds"] * 1e6 / d["requests"], "p99 us": lambda d: d["rtt_p99_us"]}),
    "echo native-poll 1/32/64": (lambda b: echo_sample(b, "native-poll", 1, 32, 64, None, 1, 3), {"req/s": lambda d: -d["requests_per_second"], "server cpu us/req": lambda d: d["server_cpu_seconds"] * 1e6 / d["requests"], "p99 us": lambda d: d["rtt_p99_us"]}),
    "echo native-poll 4/128/64": (lambda b: echo_sample(b, "native-poll", 4, 128, 64, None, 1, 3), {"req/s": lambda d: -d["requests_per_second"], "server cpu us/req": lambda d: d["server_cpu_seconds"] * 1e6 / d["requests"], "p99 us": lambda d: d["rtt_p99_us"]}),
    "echo native-poll 4/128/4096": (lambda b: echo_sample(b, "native-poll", 4, 128, 4096, None, 1, 3), {"req/s": lambda d: -d["requests_per_second"], "server cpu us/req": lambda d: d["server_cpu_seconds"] * 1e6 / d["requests"], "p99 us": lambda d: d["rtt_p99_us"]}),
    "echo 4/128/64 @220k": (lambda b: echo_sample(b, "native", 4, 128, 64, 220_000, 0.5, 2), {"server cpu s": lambda d: d["server_cpu_seconds"], "p99 us": lambda d: d["rtt_p99_us"]}),
    "echo 1/1/64 @35k": (lambda b: echo_sample(b, "native", 1, 1, 64, 35_000, 0.5, 2), {"server cpu s": lambda d: d["server_cpu_seconds"], "p99 us": lambda d: d["rtt_p99_us"]}),
    "echo 1/1/64 @1": (lambda b: echo_sample(b, "native", 1, 1, 64, 1, 0, 3), {"server cpu ms": lambda d: d["server_cpu_seconds"] * 1e3}),
    "pulse 1 Hz": (lambda b: out([f"{B}/rb-{b}/target/release/rb", "pulse", "4", "3"]), {"cpu ms": lambda d: d["cpu_ns_per_op"] * 3 / 1e6}),
    "spawn": (lambda b: out([f"{B}/rb-{b}/target/release/rb", "spawn", "4", "20000"]), {"ns/op": lambda d: d["ns_per_op"], "cpu ns/op": lambda d: d["cpu_ns_per_op"]}),
    "spawn pending": (lambda b: out([f"{B}/rb-{b}/target/release/rb", "spawn-pending", "4", "20000"]), {"ns/op": lambda d: d["ns_per_op"], "cpu ns/op": lambda d: d["cpu_ns_per_op"]}),
    "echo iouring 1/1/64 @1": (lambda b: echo_sample(b, "iouring", 1, 1, 64, 1, 0, 10), {"server cpu ms": lambda d: d["server_cpu_seconds"] * 1e3}),
    "echo native 1/1/64 @1 10s": (lambda b: echo_sample(b, "native", 1, 1, 64, 1, 0, 10), {"server cpu ms": lambda d: d["server_cpu_seconds"] * 1e3}),
    "stall 2 workers": (lambda b: out([f"{B}/rb-{b}/target/release/rb", "stall", "2", "3"]), {"late p50 ms": lambda d: d["p50"] / 1e6, "late max ms": lambda d: d["max"] / 1e6}),
}

def main():
    builds = sys.argv[1].split(","); reps = int(sys.argv[2]); only = sys.argv[3:] or list(CASES)
    rng = random.Random(1); samples = {name: {b: [] for b in builds} for name in only}
    for _ in range(reps):
        order = only[:]; rng.shuffle(order)
        for name in order:
            bs = builds[:]; rng.shuffle(bs)
            for b in bs:
                samples[name][b].append(CASES[name][0](b))
    base = builds[0]
    print(f"| Case | Metric | {' | '.join(builds)} | " + " | ".join(f"{b} vs {base}" for b in builds[1:]) + " |")
    print("| --- | --- |" + " --- |" * (2 * len(builds) - 1))
    for name in only:
        for metric, get in CASES[name][1].items():
            values = {b: [get(d) for d in samples[name][b]] for b in builds}
            med = {b: statistics.median(v) for b, v in values.items()}
            cells = [f"{med[b]:.4g} ({min(values[b]):.4g}–{max(values[b]):.4g})" for b in builds]
            changes = [f"{(med[b] - med[base]) / med[base] * 100:+.1f}%" if med[base] else "n/a" for b in builds[1:]]
            print(f"| {name} | {metric} | " + " | ".join(cells) + " | " + " | ".join(changes) + " |", flush=True)

main()
