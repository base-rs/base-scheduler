#!/usr/bin/env python3
"""Periodic timer diagnostic under oversubscription: lateness and CPU for each build, and rescue counts from a diagnostics build."""
import json, os, random, statistics, subprocess, sys
B = os.path.dirname(os.path.abspath(__file__))
builds = sys.argv[1].split(","); reps = int(sys.argv[2]); counts_binary = sys.argv[3]; output = sys.argv[4]
conditions = {
    "8 workers, idle": dict(workers=8, cpus=None, spinners=0),
    "8 workers + 4 spinners": dict(workers=8, cpus=None, spinners=4),
    "4 workers on 2 CPUs": dict(workers=4, cpus="0,1", spinners=0),
}
def spin(n):
    return [subprocess.Popen(["sh", "-c", "while :; do :; done"]) for _ in range(n)]
def run(command, cpus):
    prefix = ["taskset", "-c", cpus] if cpus else []
    return json.loads(subprocess.check_output(prefix + command, timeout=120))
rng = random.Random(7)
with open(output, "a") as out:
    for name, c in conditions.items():
        spinners = spin(c["spinners"])
        try:
            for repeat in range(reps):
                for backend in ["native", "iouring"]:
                    order = builds[:]; rng.shuffle(order)
                    for build in order:
                        data = run([f"{B}/diag-{build}/target/release/diag", backend, str(c["workers"]), "1024", "1000"], c["cpus"])
                        out.write(json.dumps({"condition": name, "backend": backend, "build": build, "repeat": repeat, "data": data}) + "\n"); out.flush()
                    if repeat < 3:
                        data = run([counts_binary, "periodic", backend, str(c["workers"]), "1024", "1000"], c["cpus"])
                        out.write(json.dumps({"condition": name, "backend": backend, "build": "counts", "repeat": repeat, "data": data}) + "\n"); out.flush()
        finally:
            for p in spinners: p.kill()
        print(f"{name} done", flush=True)
