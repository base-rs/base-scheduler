#!/usr/bin/env python3
"""Stall rescue's cost: the periodic timer diagnostic and the 1 Hz pulse for builds before stall rescue, with it on, and with it off, in random order, recording each run with its per-thread CPU time and context switches. Usage: costs.py BUILDS REPEATS OUTPUT"""
import collections, json, os, random, subprocess, sys, time
B = os.path.dirname(os.path.abspath(__file__))
builds = sys.argv[1].split(","); repeats = int(sys.argv[2]); output = sys.argv[3]

def sample(command, env=None):
    # Run a command that prints one JSON record, sampling /proc for each thread's CPU time and context switches until it exits.
    process = subprocess.Popen(command, stdout=subprocess.PIPE, env=env)
    tick = os.sysconf("SC_CLK_TCK"); last = {}
    while process.poll() is None:
        try:
            for thread in os.listdir(f"/proc/{process.pid}/task"):
                try:
                    stat = open(f"/proc/{process.pid}/task/{thread}/stat").read()
                    name = stat[stat.index("(") + 1:stat.rindex(")")]; fields = stat[stat.rindex(")") + 2:].split()
                    status = open(f"/proc/{process.pid}/task/{thread}/status").read()
                    voluntary = int(status.split("\nvoluntary_ctxt_switches:")[1].split()[0])
                    involuntary = int(status.split("nonvoluntary_ctxt_switches:")[1].split()[0])
                    last[thread] = (name, (int(fields[11]) + int(fields[12])) / tick, voluntary, involuntary)
                except (FileNotFoundError, ProcessLookupError, IndexError):
                    pass
        except FileNotFoundError:
            break
        time.sleep(0.05)
    if process.returncode:
        raise SystemExit(f"{command} failed with {process.returncode}")
    record = json.loads(process.stdout.read())
    threads = collections.defaultdict(lambda: {"cpu_seconds": 0.0, "voluntary": 0, "involuntary": 0})
    for name, cpu, voluntary, involuntary in last.values():
        entry = threads[name]; entry["cpu_seconds"] += cpu; entry["voluntary"] += voluntary; entry["involuntary"] += involuntary
    return record, dict(threads)

cases = []
for backend in ["native", "iouring"]:
    for aligned in [False, True]:
        cases.append((f"periodic {backend} 4/1024/1000us{' aligned' if aligned else ''}", lambda build, b=backend, a=aligned: sample([f"{B}/diag-{build}/target/release/diag", b, "4", "1024", "1000"], dict(os.environ, CORE_TIMER_ALIGNED="1") if a else None)))
cases.append(("pulse 1 Hz", lambda build: sample([f"{B}/rb-{build}/target/release/rb", "pulse", "4", "3"])))
rng = random.Random(20261106)
with open(output, "a") as out:
    for repeat in range(repeats):
        order = list(range(len(cases))); rng.shuffle(order)
        for index in order:
            name, run = cases[index]
            for build in rng.sample(builds, len(builds)):
                record, threads = run(build)
                out.write(json.dumps({"case": name, "build": build, "repeat": repeat, "record": record, "threads": threads}) + "\n"); out.flush()
        print(f"repeat {repeat + 1}/{repeats} complete", flush=True)
