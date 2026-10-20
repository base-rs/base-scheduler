import importlib.util,pathlib,types,json,random,hashlib,time,statistics
base=pathlib.Path('/home/samuel/Developer/core-rs/core-scheduler-perf-tokio')
spec=importlib.util.spec_from_file_location('runner',base/'benchmarks/echo/results/readiness-profiling/run.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
out=pathlib.Path('/home/samuel/.cache/core-scheduler-ring-notification-results/matched');out.mkdir(exist_ok=True)
paths={'before':pathlib.Path('/home/samuel/.cache/core-scheduler-ring-notification-results/control'),'after':out.parent/'benchmark','client':pathlib.Path('/tmp/core-scheduler-profile-baseline/release/core-scheduler-echo-benchmark')}
report={'source_before':'63da755','source_after':'63da755 + source.patch (uncommitted)','binaries':{k:{'path':str(p),'sha256':hashlib.sha256(p.read_bytes()).hexdigest()} for k,p in paths.items()},'started_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'samples':[]}
args=types.SimpleNamespace(client=paths['client'],output=out,privileged_perf=False,perf_event='cycles:u')
cases=[(v,b,r) for r in range(1,4) for v in ['before','after'] for b in ['iouring']]
random.Random(20261027).shuffle(cases)
for v,b,r in cases:
 args.server=paths[v]
 s=m.sample(args,'control',b,1,1,64,r);s['variant']=v;report['samples'].append(s)
 (out/'aiko.json').write_text(json.dumps(report,indent=2)+'\n');print(v,b,r,round(s['requests_per_second']),flush=True)
report['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());(out/'aiko.json').write_text(json.dumps(report,indent=2)+'\n')
for v,b in sorted(set((s['variant'],s['backend']) for s in report['samples'])):
 xs=[s for s in report['samples'] if (s['variant'],s['backend'])==(v,b)];print(v,b,'median',statistics.median(s['requests_per_second'] for s in xs),'switches',statistics.median(s['server_cpu']['voluntary_switches'] for s in xs),flush=True)
