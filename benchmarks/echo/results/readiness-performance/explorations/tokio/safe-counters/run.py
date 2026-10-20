from pathlib import Path
import hashlib,importlib.util,json,types
base=Path('/home/samuel/.cache/core-scheduler-tokio-next-results');root=Path('/home/samuel/Developer/core-rs/core-scheduler-perf-tokio-next')
spec=importlib.util.spec_from_file_location('runner',root/'benchmarks/echo/results/readiness-profiling/run.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
out=base/'safe-counters/samples';out.mkdir(exist_ok=False)
server=base/'safe-counters/benchmark';client=Path('/home/samuel/.cache/core-scheduler-owner-wake-final/client')
r={'purpose':'Mechanism counts only; instrumented throughput is not capacity evidence','binaries':{k:{'path':str(p),'sha256':hashlib.sha256(p.read_bytes()).hexdigest()} for k,p in [('server',server),('client',client)]},'samples':[]}
a=types.SimpleNamespace(server=server,client=client,output=out,privileged_perf=False)
for backend,size in [('tokio-adapter',64),('tokio-adapter-poll',64),('tokio-adapter',4096),('tokio-adapter-poll',4096)]:
 x=m.sample(a,'counted',backend,1,32,size,1)
 rows=[json.loads(line.split('SOCKET_COUNTERS ',1)[1]) for line in (out/(x['identifier']+'.stderr')).read_text().splitlines() if line.startswith('SOCKET_COUNTERS ')]
 assert rows
 x['counter_threads']=rows;x['counters']={k:sum(row[k] for row in rows) for k in rows[0]}
 r['samples'].append(x)
 (out/(x['identifier']+'.json')).write_text(json.dumps(x,indent=2)+'\n');(out/'aiko.json').write_text(json.dumps(r,indent=2)+'\n')
 print(backend,size,x['counters'],flush=True)
