#!/usr/bin/env python3
"""Quick paired comparisons of two base-scheduler trees on this host, for optimization work.

Each mode builds both trees, then runs every case with the builds interleaved in random order within each round, and reports the median per-round change from BASE to CANDIDATE with the number of rounds that improved. Unchanged controls (raw Tokio, Hyper) run in the same rounds as a noise check: their builds are identical, so any change they show is noise.

A layout control also runs by default: a second build of BASE from a copy at another path. Cargo hashes each package's path into its build, which moves code and data addresses, so identical source built at another path can differ by 2-3% in throughput and show 0/5 rounds better. A candidate's change counts only if it clearly exceeds the layout control's change. Disable with --no-layout-control.

The whole build and run holds /tmp/base-scheduler-perf.lock, so concurrent comparisons queue instead of disturbing each other.

  quick.py echo   BASE CANDIDATE [--workers 4] [--connections 32 256] [--backends native-poll iouring] [--rounds 5]
  quick.py http   BASE CANDIDATE [--connections 16 64 256] [--backends base-native base-iouring] [--rounds 5]
  quick.py timers BASE CANDIDATE [--workers 4 8] [--backends native iouring] [--rounds 9] [--aligned]

BASE and CANDIDATE are paths to base-scheduler source trees on this host (no target directory or .git needed). Results are appended to --output (default results/<mode>-<time>.jsonl) and summarized on stdout.
"""
import argparse, fcntl, json, os, random, re, select, shutil, statistics, subprocess, sys, tempfile, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
LOCK = "/tmp/base-scheduler-perf.lock"
WRK = "/home/samuel/Developer/ioquatix/wrk/wrk"
HTTP_SOURCE = HERE / "http-source"  # base-http at v0.3.0 with the hello benchmark's Direct backend removed and scheduler pins at 0.4.0
HARNESS = HERE / "harness"  # timer-diagnostic.rs and diag.Cargo.toml from benchmarks/timers/results/rescue-costs


def log(message):
    print(message, file=sys.stderr, flush=True)


def build(command, cwd):
    result = subprocess.run(command, cwd=cwd, capture_output=True, text=True)
    if result.returncode:
        sys.exit(f"build failed in {cwd}:\n{result.stderr[-4000:]}")


def tree_name(path):
    return Path(path).resolve().name


# ---- echo -------------------------------------------------------------------

def echo_build(tree):
    target = Path(tree) / "target-quick"
    build(["cargo", "build", "-q", "--release", "-p", "base-scheduler-benchmark-echo", "--target-dir", str(target)], tree)
    binaries = [p for p in (target / "release").iterdir() if p.is_file() and os.access(p, os.X_OK) and "echo" in p.name and not p.name.endswith(".d")]
    return min(binaries, key=lambda p: len(p.name))


def echo_sample(server_binary, client_binary, backend, workers, connections, args):
    server_cpus = ",".join(map(str, range(workers)))
    server = subprocess.Popen(["taskset", "-c", server_cpus, str(server_binary), "server", backend, str(workers), str(connections)], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    try:
        if not select.select([server.stdout], [], [], 30)[0]:
            raise RuntimeError("echo server did not start")
        address = server.stdout.readline().strip()
        client = subprocess.run(["taskset", "-c", "4-7", str(client_binary), "client", address, "4", str(connections), str(args.bytes), "1", str(args.seconds)], capture_output=True, text=True, timeout=args.seconds + 40, check=True)
        _, status, usage = os.wait4(server.pid, 0)
        server.returncode = os.waitstatus_to_exitcode(status)
        data = json.loads(client.stdout)
        return {"requests_per_second": data["requests_per_second"], "rtt_p50_us": data["rtt_p50_us"], "rtt_p99_us": data["rtt_p99_us"],
                "cpu_us_per_request": (usage.ru_utime + usage.ru_stime) / data["requests"] * 1e6}
    finally:
        if server.poll() is None:
            server.kill()
            server.wait()


def echo(args, out):
    binaries = {}
    for name, tree in args.trees.items():
        log(f"building echo in {tree}")
        binaries[name] = echo_build(tree)
    client = binaries["base"]  # one fixed client for every sample
    controls = [c for c in ["tokio"] if c not in args.backends]
    cases = [(w, c) for w in args.workers for c in args.connections]
    for round_ in range(args.rounds):
        for workers, connections in random.sample(cases, len(cases)):
            runs = [(name, backend) for name in args.trees for backend in args.backends] + [("base", backend) for backend in controls]
            for name, backend in random.sample(runs, len(runs)):
                record = echo_sample(binaries[name], client, backend, workers, connections, args)
                emit(out, {"mode": "echo", "round": round_, "case": f"{workers}w {connections}c", "build": name, "backend": backend, **record})
        log(f"round {round_ + 1}/{args.rounds}")
    return ["requests_per_second", "cpu_us_per_request", "rtt_p99_us"]


# ---- http -------------------------------------------------------------------

def http_build(name, tree):
    source = HERE / "http-trees" / name
    if source.exists():
        shutil.rmtree(source)
    shutil.copytree(HTTP_SOURCE, source, ignore=shutil.ignore_patterns("target"))
    tree = Path(tree).resolve()
    patch = source / "patch.toml"
    crates = {"base-scheduler": tree, **{f"base-scheduler-{c}": tree / "crates" / c for c in ["native", "selector", "tokio", "iouring", "timers"]}}
    patch.write_text("[patch.crates-io]\n" + "".join(f'{k} = {{ path = "{v}" }}\n' for k, v in crates.items()))
    hello = source / "benchmarks" / "hello"
    build(["cargo", "build", "-q", "--release", "--config", str(patch), "--target-dir", str(tree / "target-quick-http")], hello)
    return tree / "target-quick-http" / "release" / "base-http-hello-benchmark"


def http_sample(binary, backend, connections, args):
    sys.path.insert(0, str(HTTP_SOURCE / "benchmarks" / "hello"))
    from run import validate, parse_wrk
    server = subprocess.Popen(["taskset", "-c", "0", str(binary), backend], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    try:
        if not select.select([server.stdout], [], [], 10)[0]:
            raise RuntimeError("http server did not start")
        address = server.stdout.readline().strip()
        validate(address, raw=False)
        command = ["taskset", "-c", "2,3,6,7", WRK, f"-t{1 if connections == 1 else 4}", f"-c{connections}", "--latency", "--timeout", "2s"]
        subprocess.run(command + ["-d1s", f"http://{address}/"], capture_output=True, text=True, check=True, timeout=20)
        tick = os.sysconf("SC_CLK_TCK")
        cpu = lambda: sum(int(v) for v in open(f"/proc/{server.pid}/stat").read().rsplit(")", 1)[1].split()[11:13]) / tick
        before = cpu()
        wrk = subprocess.run(command + [f"-d{args.seconds}s", f"http://{address}/"], capture_output=True, text=True, check=True, timeout=args.seconds + 20)
        used = cpu() - before
        errors = re.search(r"timeout (\d+)", wrk.stdout)
        result = parse_wrk(re.sub(r"\n\s*Socket errors:.*", "", wrk.stdout))
        requests = int(re.search(r"(\d+) requests in", wrk.stdout)[1])
        return {"requests_per_second": result["requests_per_second"], "rtt_p50_us": result["latency_p50_ms"] * 1000, "rtt_p99_us": result["latency_p99_ms"] * 1000,
                "cpu_us_per_request": used / requests * 1e6, "timeouts": int(errors[1]) if errors else 0}
    finally:
        server.kill()
        server.wait()


def http(args, out):
    binaries = {}
    for name, tree in args.trees.items():
        log(f"building http hello against {tree}")
        binaries[name] = http_build(name, tree)
    controls = ["hyper-tokio"]
    for round_ in range(args.rounds):
        for connections in random.sample(args.connections, len(args.connections)):
            runs = [(name, backend) for name in args.trees for backend in args.backends] + [("base", backend) for backend in controls]
            for name, backend in random.sample(runs, len(runs)):
                record = http_sample(binaries[name], backend, connections, args)
                emit(out, {"mode": "http", "round": round_, "case": f"{connections}c", "build": name, "backend": backend, **record})
        log(f"round {round_ + 1}/{args.rounds}")
    return ["requests_per_second", "cpu_us_per_request", "rtt_p99_us"]


# ---- timers -----------------------------------------------------------------

def timers_build(name, tree):
    package = HERE / "timer-trees" / name
    (package / "src").mkdir(parents=True, exist_ok=True)
    shutil.copy(HARNESS / "timer-diagnostic.rs", package / "src" / "main.rs")
    (package / "Cargo.toml").write_text((HARNESS / "diag.Cargo.toml").read_text().replace("TREE", str(Path(tree).resolve())))
    build(["cargo", "build", "-q", "--release"], package)
    return package / "target" / "release" / "diag"


def timers(args, out):
    binaries = {}
    for name, tree in args.trees.items():
        log(f"building the periodic timer diagnostic against {tree}")
        binaries[name] = timers_build(name, tree)
    environment = dict(os.environ, CORE_TIMER_ALIGNED="1") if args.aligned else None
    cases = [(w, b) for w in args.workers for b in args.backends]
    for round_ in range(args.rounds):
        for workers, backend in random.sample(cases, len(cases)):
            for name in random.sample(list(args.trees), len(args.trees)):
                data = json.loads(subprocess.check_output([str(binaries[name]), backend, str(workers), str(args.tasks), str(args.delay)], env=environment, timeout=120))
                emit(out, {"mode": "timers", "round": round_, "case": f"{workers}w", "build": name, "backend": backend,
                           "lateness_p50_us": data["lateness_p50_us"], "lateness_p99_us": data["lateness_p99_us"], "cpu_seconds": data["cpu_seconds"]})
        log(f"round {round_ + 1}/{args.rounds}")
    return ["lateness_p50_us", "lateness_p99_us", "cpu_seconds"]


# ---- summary ----------------------------------------------------------------

def emit(out, record):
    out.write(json.dumps(record) + "\n")
    out.flush()


def summarize(path, metrics):
    rows = [json.loads(line) for line in open(path)]
    by = {}
    for r in rows:
        by.setdefault((r["case"], r["backend"], r["build"]), {})[r["round"]] = r
    lower_is_better = {"cpu_us_per_request", "rtt_p99_us", "lateness_p50_us", "lateness_p99_us", "cpu_seconds"}
    layout = any(k[2] == "copy" for k in by)
    print(f"\nResults: {path}\n")
    print("Change is the median of per-round changes from base; 'better' counts rounds that improved on base." + (" 'layout' is identical base source built at another path: changes of that size are build-layout noise." if layout else "") + "\n")
    columns = ["candidate"] + (["layout"] if layout else [])
    print("| Case | Backend | " + " | ".join(f"{m} base | cand | " + " | ".join(f"{c} change (better)" for c in columns) for m in metrics) + " |")
    print("| --- | --- | " + " | ".join("---: | ---: | " + " | ".join("---:" for _ in columns) for _ in metrics) + " |")
    def change(base, other, m):
        changes = [other[k][m] / base[k][m] - 1 for k in other if k in base and base[k][m]]
        better = sum(1 for v in changes if (v < 0) == (m in lower_is_better) and v != 0)
        return f"{statistics.median(changes) * 100:+.1f}% ({better}/{len(changes)})"
    for case, backend in sorted({(k[0], k[1]) for k in by}):
        base = by.get((case, backend, "base"), {})
        cand = by.get((case, backend, "candidate"), {})
        cells = []
        for m in metrics:
            b = statistics.median(r[m] for r in base.values()) if base else float("nan")
            if not cand:
                cells.append(f"{b:.4g} | control | " + " | ".join("—" for _ in columns))
                continue
            c = statistics.median(r[m] for r in cand.values())
            parts = [change(base, cand, m)]
            if layout:
                copy = by.get((case, backend, "copy"), {})
                parts.append(change(base, copy, m) if copy else "—")
            cells.append(f"{b:.4g} | {c:.4g} | " + " | ".join(parts))
        print(f"| {case} | {backend} | " + " | ".join(cells) + " |")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("mode", choices=["echo", "http", "timers"])
    parser.add_argument("base")
    parser.add_argument("candidate")
    parser.add_argument("--workers", type=int, nargs="+")
    parser.add_argument("--connections", type=int, nargs="+")
    parser.add_argument("--backends", nargs="+")
    parser.add_argument("--rounds", type=int)
    parser.add_argument("--seconds", type=int)
    parser.add_argument("--bytes", type=int, default=64)
    parser.add_argument("--tasks", type=int, default=1024)
    parser.add_argument("--delay", type=int, default=1000)
    parser.add_argument("--aligned", action="store_true")
    parser.add_argument("--seed", type=int, default=20261106)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--no-layout-control", action="store_true")
    args = parser.parse_args()
    defaults = {
        "echo": dict(workers=[4], connections=[32, 256], backends=["native-poll", "iouring"], rounds=5, seconds=3),
        "http": dict(workers=[1], connections=[16, 64, 256], backends=["base-native", "base-iouring"], rounds=5, seconds=4),
        "timers": dict(workers=[4, 8], connections=[], backends=["native", "iouring"], rounds=9, seconds=3),
    }[args.mode]
    for key, value in defaults.items():
        if getattr(args, key) is None:
            setattr(args, key, value)
    args.trees = {"base": str(Path(args.base).resolve()), "candidate": str(Path(args.candidate).resolve())}
    for tree in args.trees.values():
        if not (Path(tree) / "crates" / "native").is_dir():
            sys.exit(f"not a base-scheduler tree: {tree}")
    if not args.no_layout_control:
        # Identical source at a different path: a different crate hash and code layout.
        copy = HERE / "layout" / f"{Path(args.trees['base']).name}-layout"
        copy.parent.mkdir(exist_ok=True)
        subprocess.run(["rsync", "-a", "--delete", "--exclude", "target*", "--exclude", ".git", args.trees["base"] + "/", str(copy) + "/"], check=True)
        args.trees["copy"] = str(copy)
    random.seed(args.seed)
    output = args.output or HERE / "results" / f"{args.mode}-{tree_name(args.candidate)}-{time.strftime('%Y%m%d-%H%M%S')}.jsonl"
    output.parent.mkdir(parents=True, exist_ok=True)
    with open(LOCK, "w") as lock:
        log(f"waiting for {LOCK}")
        fcntl.flock(lock, fcntl.LOCK_EX)
        log("lock held")
        with open(output, "a") as out:
            metrics = {"echo": echo, "http": http, "timers": timers}[args.mode](args, out)
    summarize(output, metrics)


if __name__ == "__main__":
    main()
