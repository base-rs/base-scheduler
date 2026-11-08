#!/usr/bin/env python3
"""Multi-worker HTTP hello: Base (Native epoll and io_uring) against Hyper on a multi-thread Tokio runtime.

Builds the private hello benchmark copy (mw/http-source, with the `-mt` modes in src/mw.rs) against a base-scheduler tree and a layout replicate of it (identical source at another path), then runs every case in interleaved random order within each round. Server on CPUs 0..N-1, wrk on the CPUs listed per N. wrk socket errors, timeouts and non-2xx responses are recorded per sample, not fatal.

  mw.py TREE [--candidate TREE] [--workers 2 4] [--connections 16 64 256] [--rounds 5] [--seconds 4] [--seed 20261106]

Holds /tmp/base-scheduler-perf.lock around the builds and runs. Reuses quick.py's helpers.
"""
import argparse, fcntl, json, os, random, re, select, shutil, statistics, subprocess, sys, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import quick  # noqa: E402  (log, build, emit, LOCK, WRK)

SOURCE = HERE / "http-source"
sys.path.insert(0, str(SOURCE / "benchmarks" / "hello"))
from run import validate, parse_wrk  # noqa: E402

# Server on 0..N-1; wrk on the rest. N=4 leaves wrk on 4-7, the SMT siblings of the server's CPUs.
LAYOUT = {1: ("0", "2,3,6,7"), 2: ("0,1", "2,3,6,7"), 4: ("0-3", "4-7")}
BASE_BACKENDS = ["base-native-mt", "base-iouring-mt"]
CONTROL = "hyper-tokio-mt"


def build(name, tree):
    tree = Path(tree).resolve()
    patch = HERE / "patches" / f"{name}.toml"
    patch.parent.mkdir(exist_ok=True)
    crates = {"base-scheduler": tree, **{f"base-scheduler-{c}": tree / "crates" / c for c in ["native", "selector", "tokio", "iouring", "timers"]}}
    patch.write_text("[patch.crates-io]\n" + "".join(f'{k} = {{ path = "{v}" }}\n' for k, v in crates.items()))
    target = HERE / "targets" / name
    # A patched build rewrites the lockfile; start each build from the shared benchmark's lockfile, as quick.py's fresh copies do.
    shutil.copy(HERE.parent / "http-source" / "benchmarks" / "hello" / "Cargo.lock", SOURCE / "benchmarks" / "hello" / "Cargo.lock")
    quick.build(["cargo", "build", "-q", "--release", "--config", str(patch), "--target-dir", str(target)], SOURCE / "benchmarks" / "hello")
    return target / "release" / "base-http-hello-benchmark"


def context_switches(pid):
    total = 0
    for task in os.listdir(f"/proc/{pid}/task"):
        try:
            for line in open(f"/proc/{pid}/task/{task}/status"):
                if line.startswith(("voluntary_ctxt_switches", "nonvoluntary_ctxt_switches")):
                    total += int(line.split()[1])
        except FileNotFoundError:
            pass
    return total


def sample(binary, backend, workers, connections, seconds):
    server_cpus, client_cpus = LAYOUT[workers]
    server = subprocess.Popen(["taskset", "-c", server_cpus, str(binary), backend, str(workers)], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    try:
        if not select.select([server.stdout], [], [], 10)[0]:
            return {"failure": "server did not start"}
        address = server.stdout.readline().strip()
        validate(address, raw=False)
        command = ["taskset", "-c", client_cpus, quick.WRK, f"-t{min(4, connections)}", f"-c{connections}", "--latency", "--timeout", "2s"]
        subprocess.run(command + ["-d1s", f"http://{address}/"], capture_output=True, text=True, timeout=20)
        tick = os.sysconf("SC_CLK_TCK")
        cpu = lambda: sum(int(v) for v in open(f"/proc/{server.pid}/stat").read().rsplit(")", 1)[1].split()[11:13]) / tick
        before, switches = cpu(), context_switches(server.pid)
        wrk = subprocess.run(command + [f"-d{seconds}s", f"http://{address}/"], capture_output=True, text=True, timeout=seconds + 20)
        used, switches = cpu() - before, context_switches(server.pid) - switches
        if wrk.returncode or "requests in" not in wrk.stdout:
            return {"failure": f"wrk exited {wrk.returncode}: {wrk.stdout[-500:]} {wrk.stderr[-500:]}"}
        errors = re.search(r"Socket errors: connect (\d+), read (\d+), write (\d+), timeout (\d+)", wrk.stdout)
        non2xx = re.search(r"Non-2xx or 3xx responses: (\d+)", wrk.stdout)
        clean = re.sub(r"\n\s*(Socket errors|Non-2xx or 3xx responses):.*", "", wrk.stdout)
        result = parse_wrk(clean)
        requests = int(re.search(r"(\d+) requests in", wrk.stdout)[1])
        record = {"requests_per_second": result["requests_per_second"], "rtt_p50_us": result["latency_p50_ms"] * 1000, "rtt_p99_us": result["latency_p99_ms"] * 1000,
                  "cpu_us_per_request": used / requests * 1e6, "switches_per_request": switches / requests, "requests": requests,
                  "connect_errors": 0, "read_errors": 0, "write_errors": 0, "timeouts": 0, "non2xx": int(non2xx[1]) if non2xx else 0}
        if errors:
            record.update(dict(zip(["connect_errors", "read_errors", "write_errors", "timeouts"], map(int, errors.groups()))))
        if server.poll() is not None:
            record["failure"] = f"server exited {server.returncode}"
        return record
    except Exception as error:  # record and continue: one failed sample must not lose the run
        return {"failure": repr(error)}
    finally:
        if server.poll() is None:
            server.kill()
        server.wait()


def summarize(path):
    rows = [json.loads(line) for line in open(path)]
    by = {}
    for r in rows:
        by.setdefault((r["workers"], r["connections"], r["backend"], r["build"]), {})[r["round"]] = r
    print(f"\nResults: {path}\n")
    print("Medians over rounds. 'vs Hyper' is the median of per-round ratios to the same round's Hyper sample (better: rounds where Base wins). 'layout' is identical base source built at another path: its change from base is the build-layout noise floor.\n")
    metrics = [("requests_per_second", "req/s", False), ("cpu_us_per_request", "CPU µs/req", True), ("rtt_p50_us", "p50 µs", True), ("rtt_p99_us", "p99 µs", True)]
    print("| N | Conns | Backend | Build | " + " | ".join(f"{label} | vs Hyper (better)" for _, label, _ in metrics) + " | ctx sw/req | timeouts (total) | other errors | failures |")
    print("| ---: | ---: | --- | --- | " + " | ".join("---: | ---:" for _ in metrics) + " | ---: | ---: | ---: | ---: |")
    for (workers, connections, backend, build_) in sorted(by, key=lambda k: (k[0], k[1], k[2] == CONTROL, k[2], k[3])):
        samples = by[(workers, connections, backend, build_)]
        good = {k: r for k, r in samples.items() if "failure" not in r}
        hyper = {k: r for k, r in by.get((workers, connections, CONTROL, "base"), {}).items() if "failure" not in r}
        cells = []
        for m, _, lower in metrics:
            if not good:
                cells.append("— | —")
                continue
            value = statistics.median(r[m] for r in good.values())
            if backend == CONTROL:
                cells.append(f"{value:.4g} | control")
                continue
            ratios = [good[k][m] / hyper[k][m] - 1 for k in good if k in hyper and hyper[k][m]]
            better = sum(1 for v in ratios if (v < 0) == lower and v != 0)
            cells.append(f"{value:.4g} | " + (f"{statistics.median(ratios) * 100:+.1f}% ({better}/{len(ratios)})" if ratios else "—"))
        switches = f"{statistics.median(r['switches_per_request'] for r in good.values()):.3f}" if good else "—"
        timeouts = sum(r["timeouts"] for r in good.values())
        other = sum(r["connect_errors"] + r["read_errors"] + r["write_errors"] + r["non2xx"] for r in good.values())
        failures = len(samples) - len(good)
        print(f"| {workers} | {connections} | {backend} | {build_} | " + " | ".join(cells) + f" | {switches} | {timeouts} | {other} | {failures} |")
    # Layout noise floor on the Base backends.
    print("\nChange from base (median per-round, rounds better); 'copy' is the layout replicate:\n")
    for (workers, connections, backend, build_) in sorted(by, key=lambda k: (k[0], k[1], k[2], k[3] != "candidate")):
        if build_ == "base":
            continue
        base = {k: r for k, r in by.get((workers, connections, backend, "base"), {}).items() if "failure" not in r}
        copy = {k: r for k, r in by[(workers, connections, backend, build_)].items() if "failure" not in r}
        parts = []
        for m, label, lower in metrics:
            ratios = [copy[k][m] / base[k][m] - 1 for k in copy if k in base and base[k][m]]
            if ratios:
                better = sum(1 for v in ratios if (v < 0) == lower and v != 0)
                parts.append(f"{label} {statistics.median(ratios) * 100:+.1f}% ({better}/{len(ratios)})")
        print(f"- N={workers} {connections}c {backend} {build_}: " + ", ".join(parts))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("tree", nargs="?")
    parser.add_argument("--candidate", help="a second tree, reported as a change from TREE beside the layout replicate")
    parser.add_argument("--workers", type=int, nargs="+", default=[2, 4])
    parser.add_argument("--connections", type=int, nargs="+", default=[16, 64, 256])
    parser.add_argument("--backends", nargs="+", default=BASE_BACKENDS)
    parser.add_argument("--rounds", type=int, default=5)
    parser.add_argument("--seconds", type=int, default=4)
    parser.add_argument("--seed", type=int, default=20261106)
    parser.add_argument("--no-layout-control", action="store_true")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--summarize", type=Path, help="only summarize an existing results file")
    args = parser.parse_args()
    if args.summarize:
        return summarize(args.summarize)
    trees = {"base": str(Path(args.tree).resolve())}
    if args.candidate:
        trees["candidate"] = str(Path(args.candidate).resolve())
    if not args.no_layout_control:
        copy = HERE / "layout" / f"{Path(trees['base']).name}-layout"
        copy.parent.mkdir(exist_ok=True)
        subprocess.run(["rsync", "-a", "--delete", "--exclude", "target*", "--exclude", ".git", trees["base"] + "/", str(copy) + "/"], check=True)
        trees["copy"] = str(copy)
    random.seed(args.seed)
    output = args.output or HERE / "results" / f"mw-{Path(trees.get('candidate', trees['base'])).name}-{time.strftime('%Y%m%d-%H%M%S')}.jsonl"
    output.parent.mkdir(parents=True, exist_ok=True)
    with open(quick.LOCK, "w") as lock:
        quick.log(f"waiting for {quick.LOCK}")
        fcntl.flock(lock, fcntl.LOCK_EX)
        quick.log("lock held")
        binaries = {}
        for name, tree in trees.items():
            quick.log(f"building multi-worker hello against {tree}")
            binaries[name] = build(name, tree)
        cases = [(w, c) for w in args.workers for c in args.connections]
        with open(output, "a") as out:
            for round_ in range(args.rounds):
                for workers, connections in random.sample(cases, len(cases)):
                    runs = [(name, b) for name in trees for b in args.backends] + [("base", CONTROL)]
                    for name, backend in random.sample(runs, len(runs)):
                        record = sample(binaries[name], backend, workers, connections, args.seconds)
                        if "failure" in record:
                            quick.log(f"sample failed: N={workers} {connections}c {name} {backend}: {record['failure']}")
                        quick.emit(out, {"round": round_, "workers": workers, "connections": connections, "build": name, "backend": backend, **record})
                quick.log(f"round {round_ + 1}/{args.rounds}")
    summarize(output)


if __name__ == "__main__":
    main()
