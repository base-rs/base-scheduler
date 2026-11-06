import json, statistics, sys
rows=[json.loads(l) for l in open(sys.argv[1])]
pairs=[p.split(":") for p in sys.argv[2:]]
def pair(w, base, cand, f):
    by={}
    for r in rows:
        if r["workers"]==w: by.setdefault(r["round"],{})[r["build"]]=r[f]
    ch=[v[cand]/v[base]-1 for v in by.values() if cand in v and base in v]
    return f"{statistics.median(ch)*100:+.1f}% ({sum(c<0 for c in ch)}/{len(ch)} lower)"
def med(w,b,f):
    v=[r[f] for r in rows if r["workers"]==w and r["build"]==b]; return f"{statistics.median(v):.4g} ({min(v):.3g}–{max(v):.3g})"
builds=list(dict.fromkeys(r["build"] for r in rows))
for w in sorted({r["workers"] for r in rows}):
    print(f"\n{w} workers, {rows[0]['backend']}\n\n| build | p50 µs | p99 µs | CPU s |\n| --- | ---: | ---: | ---: |")
    for b in builds:
        print(f"| {b} | {med(w,b,'lateness_p50_us')} | {med(w,b,'lateness_p99_us')} | {med(w,b,'cpu_seconds')} |")
    print("\n| pair | p50 | p99 | CPU |\n| --- | ---: | ---: | ---: |")
    for base,cand in pairs:
        print(f"| {cand} vs {base} | {pair(w,base,cand,'lateness_p50_us')} | {pair(w,base,cand,'lateness_p99_us')} | {pair(w,base,cand,'cpu_seconds')} |")
