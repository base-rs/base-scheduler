from pathlib import Path
import hashlib,importlib.util,json,types,random,statistics
base=Path('/home/samuel/.cache/core-scheduler-tokio-next-results');root=Path('/home/samuel/Developer/core-rs/core-scheduler-perf-tokio-next')
spec=importlib.util.spec_from_file_location('runner',root/'benchmarks/echo/results/readiness-profiling/run.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
out=base/'short-read/screen';out.mkdir(exist_ok=False)
servers={v:base/v/'benchmark' for v in ['baseline','short-read']};client=Path('/home/samuel/.cache/core-scheduler-owner-wake-final/client')
r={'source':'427be5d + short-read/source.patch','binaries':{k:{'path':str(p),'sha256':hashlib.sha256(p.read_bytes()).hexdigest()} for k,p in [*servers.items(),('client',client)]},'samples':[]}
cases=[(v,b,i) for i in range(1,4) for v,b in [('short-read','tokio'),('baseline','tokio-adapter'),('short-read','tokio-adapter'),('baseline','tokio-adapter-poll'),('short-read','tokio-adapter-poll')]];random.Random(20261024).shuffle(cases)
for variant,backend,repeat in cases:
 a=types.SimpleNamespace(server=servers[variant],client=client,output=out,privileged_perf=False)
 x=m.sample(a,variant,backend,1,32,64,repeat);r['samples'].append(x)
 (out/(x['identifier']+'.json')).write_text(json.dumps(x,indent=2)+'\n');(out/'aiko.json').write_text(json.dumps(r,indent=2)+'\n')
 print(x['identifier'],round(x['requests_per_second']),flush=True)
for variant,backend in sorted({(x['mode'],x['backend']) for x in r['samples']}):
 print(variant,backend,round(statistics.median(x['requests_per_second'] for x in r['samples'] if x['mode']==variant and x['backend']==backend)),flush=True)
