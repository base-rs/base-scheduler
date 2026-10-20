import importlib.util,json,random,argparse,statistics,hashlib
from pathlib import Path
root=Path('/home/samuel/Developer/core-rs/core-scheduler-perf-idle')
spec=importlib.util.spec_from_file_location('runner',root/'benchmarks/echo/results/readiness-profiling/run.py')
r=importlib.util.module_from_spec(spec);spec.loader.exec_module(r)
out=Path('/home/samuel/.cache/core-scheduler-perf-idle-fix/stream-screen');out.mkdir(exist_ok=False)
bins={'before':Path('/tmp/core-perf-socket-control'),'after':Path('/tmp/core-perf-socket-candidate'),'stream':Path('/tmp/core-perf-idle-fix-stream')}
a=argparse.Namespace(server=None,client=Path('/tmp/core-scheduler-profile-baseline/release/core-scheduler-echo-benchmark'),output=out,privileged_perf=False)
samples=[]
cases=[(rep,1,c,64,backend,label) for rep in range(1,4) for c in [1] for backend,label in [(b,v) for b in ['iouring','iouring-poll'] for v in bins]+[('tokio','before')]]
random.Random(20261029).shuffle(cases)
for rep,w,c,size,backend,label in cases:
 a.server=bins[label];x=r.sample(a,label,backend,w,c,size,rep);x['variant']=label;samples.append(x);(out/'aiko.json').write_text(json.dumps({'hashes':{k:hashlib.sha256(v.read_bytes()).hexdigest() for k,v in bins.items()},'samples':samples},indent=2));print(label,x['identifier'],round(x['requests_per_second']),flush=True)
for c in [1]:
 for b in ['tokio','iouring','iouring-poll']:
  for v in bins:
   rows=[x for x in samples if x['variant']==v and x['backend']==b and x['connections']==c]
   if rows: print(c,b,v,round(statistics.median(x['requests_per_second'] for x in rows)),round(statistics.median(x['server_cpu']['user']+x['server_cpu']['system'] for x in rows),3),statistics.median(x['rtt_p99_us'] for x in rows),flush=True)
