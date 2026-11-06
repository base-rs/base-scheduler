#!/usr/bin/env python3
"""run.py OUT ROUNDS BACKEND WORKERS(comma) BUILD=BINARY... : interleaved runs of diagnostic binaries, JSON lines to OUT."""
import json, random, subprocess, sys, os

out, rounds, backend, workers = sys.argv[1], int(sys.argv[2]), sys.argv[3], [int(w) for w in sys.argv[4].split(",")]
builds = dict(arg.split("=", 1) for arg in sys.argv[5:])
env = dict(os.environ)
random.seed(7)
with open(out, "a") as f:
    for r in range(rounds):
        for w in random.sample(workers, len(workers)):
            for name in random.sample(list(builds), len(builds)):
                binary, *options = builds[name].split("@")
                extra = dict(CORE_TIMER_WORK=options[0] if options and options[0] else "0")
                if "spread" in options:
                    extra["CORE_TIMER_SPREAD"] = "1"
                data = json.loads(subprocess.check_output([binary, backend, str(w), "1024", "1000"], env=dict(env, **extra), timeout=120))
                data.update(build=name, round=r)
                f.write(json.dumps(data) + "\n")
                f.flush()
        print(f"round {r + 1}/{rounds}", file=sys.stderr, flush=True)
