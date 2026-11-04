#!/usr/bin/env python3
"""Run a benchmark command that prints one JSON record with `rescues`, and print its record with the worker threads' involuntary context switches added, sampled from /proc every 20 ms."""
import json, os, subprocess, sys, time, collections
p = subprocess.Popen(sys.argv[1:], stdout=subprocess.PIPE)
last = {}
while p.poll() is None:
    try:
        for t in os.listdir(f"/proc/{p.pid}/task"):
            try:
                st = open(f"/proc/{p.pid}/task/{t}/status").read()
                name = st.split("Name:")[1].split()[0]
                inv = int(st.split("nonvoluntary_ctxt_switches:")[1].split()[0])
                last[t] = (name, inv)
            except (FileNotFoundError, ProcessLookupError, IndexError): pass
    except FileNotFoundError: break
    time.sleep(0.02)
d = json.loads(p.stdout.read())
inv = collections.Counter()
for name, n in last.values(): inv[name] += n
print(json.dumps(dict(d, worker_involuntary_switches=inv.get("base-native-tas", 0))))
