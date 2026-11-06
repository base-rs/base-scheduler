#!/usr/bin/env python3
# usage: screen.py BINARY CLIENT REPS WORKERS CONNS BACKENDS(comma) POLICIES(comma)
import json, os, random, select, statistics, subprocess, sys
binary, client, reps, workers, conns, backends, policies = sys.argv[1:8]
reps, workers = int(reps), int(workers)
conns = [int(c) for c in conns.split(",")]
backends = backends.split(","); policies = policies.split(",")
results = {}
def sample(backend, policy, c):
    env = dict(os.environ, BASE_WD_POLICY=policy)
    cpus = ",".join(map(str, range(workers)))
    server = subprocess.Popen(["taskset", "-c", cpus, binary, "server", backend, str(workers), str(c)], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, env=env)
    select.select([server.stdout], [], [], 30)
    address = server.stdout.readline().strip()
    out = subprocess.run(["taskset", "-c", "4-7", client, "client", address, "4", str(c), "64", "1", "3"], capture_output=True, text=True, check=True)
    rest = server.stdout.read()
    _, status, usage = os.wait4(server.pid, 0)
    data = json.loads(out.stdout)
    work = {}
    try:
        work = json.loads(rest.strip().splitlines()[-1])["rescues"]["work"]
    except Exception:
        pass
    polls = work.get("polls", 1) or 1
    record = {"rps": data["requests_per_second"], "user": usage.ru_utime / data["requests"] * 1e6, "sys": usage.ru_stime / data["requests"] * 1e6, "cpu": (usage.ru_utime + usage.ru_stime) / data["requests"] * 1e6,
              "nvcsw": usage.ru_nvcsw / data["requests"], "nivcsw": usage.ru_nivcsw / data["requests"]}
    for k in ["migrations", "schedule_remote_worker", "wakes_schedule", "wakes_batch", "parks", "steal_ready", "steal_incoming"]:
        if k in work: record[k] = work[k] / polls
    return record
for r in range(reps):
    runs = [(b, p, c) for b in backends for p in policies for c in conns]
    random.shuffle(runs)
    for run in runs:
        results.setdefault(run, []).append(sample(*run))
for (b, p, c), rs in sorted(results.items(), key=lambda kv: (kv[0][2], kv[0][0], kv[0][1])):
    keys = rs[0].keys()
    print(f"{c:4}c {b:12} p{p}: " + " ".join(f"{k}={statistics.median(x[k] for x in rs):.4g}" for k in keys))
