import sys, statistics as s, collections
d=collections.defaultdict(list)
for l in open(sys.argv[1]):
    if l.startswith('#') or not l.strip(): continue
    p=l.split(); v={'rps':float(p[3])}
    for q in p[4:]:
        k,x=q.split('='); v[k.split(':')[0].split('.')[-1]]=float(x.rstrip('us'))
    d[(int(p[2]),p[0]+':'+p[1])].append(v)
for k in sorted(d):
    rows=d[k]; m={f:s.median(r[f] for r in rows) for f in rows[0]}
    print(f"{k[0]:>4} {k[1]:<18} n={len(rows)} rps={m['rps']/1e3:7.1f}k [{min(r['rps'] for r in rows)/1e3:.1f}-{max(r['rps'] for r in rows)/1e3:.1f}] " + ' '.join(f"{f}={m[f]:.1f}" for f in m if f!='rps'))
