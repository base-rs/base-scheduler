#!/usr/bin/env python3
"""Timers fired per expiry pass in the periodic diagnostic, for builds instrumented with batches-old.patch and batches-new.patch, in random order. Usage: batches-record.py BUILDS REPEATS OUTPUT WORKERS..."""
import json, random, subprocess, sys
builds = sys.argv[1].split(","); repeats = int(sys.argv[2]); output = sys.argv[3]; workers = sys.argv[4:]
rng = random.Random(20261106)
with open(output, "a") as out:
    for repeat in range(repeats):
        for count in workers:
            for build in rng.sample(builds, len(builds)):
                record = json.loads(subprocess.check_output([f"diag-{build}/target/release/diag", "native", count, "1024", "1000"], timeout=120))
                out.write(json.dumps({"build": build, "workers": int(count), "repeat": repeat, "record": record}) + "\n"); out.flush()
