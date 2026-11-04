#!/usr/bin/env python3
"""Run a command and report each thread name's CPU time and voluntary context switches, sampled from /proc every 50 ms."""
import subprocess, time, os, sys, collections
cmd = sys.argv[1:]
p = subprocess.Popen(cmd, stdout=subprocess.PIPE)
tick = os.sysconf("SC_CLK_TCK"); last = {}
while p.poll() is None:
    try:
        for t in os.listdir(f"/proc/{p.pid}/task"):
            try:
                s = open(f"/proc/{p.pid}/task/{t}/stat").read()
                name = s[s.index("(")+1:s.rindex(")")]; f = s[s.rindex(")")+2:].split()
                last[t] = (name, (int(f[11]) + int(f[12])) / tick, int(open(f"/proc/{p.pid}/task/{t}/status").read().split("voluntary_ctxt_switches:")[1].split()[0]))
            except (FileNotFoundError, ProcessLookupError, IndexError): pass
    except FileNotFoundError: break
    time.sleep(0.05)
print(p.stdout.read().decode().strip())
agg = collections.defaultdict(lambda: [0, 0])
for name, cpu, cs in last.values(): agg[name][0] += cpu; agg[name][1] += cs
for name, (cpu, cs) in agg.items(): print(f"  {name}: cpu {cpu:.2f}s voluntary switches {cs}")
