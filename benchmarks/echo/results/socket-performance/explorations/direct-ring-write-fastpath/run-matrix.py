import importlib.util,pathlib,types,json,random,hashlib,time,statistics
base=pathlib.Path('/home/samuel/Developer/core-rs/core-scheduler-perf-tokio')
spec=importlib.util.spec_from_file_location('runner',base/'benchmarks/echo/results/readiness-profiling/run.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
out=pathlib.Path('/home/samuel/.cache/core-scheduler-direct-notification-write-fastpath/matrix');out.mkdir(exist_ok=True)
paths={'before':pathlib.Path('/home/samuel/.cache/core-scheduler-direct-notification-write-fastpath/control'),'after':out.parent/'benchmark','client':pathlib.Path('/tmp/core-scheduler-profile-baseline/release/core-scheduler-echo-benchmark')}
report={'source_before':'63da755','source_after':'63da755 + source.patch (uncommitted)','binaries':{k:{'path':str(p),'sha256':hashlib.sha256(p.read_bytes()).hexdigest()} for k,p in paths.items()},'started_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'samples':[]}
args=types.SimpleNamespace(client=paths['client'],output=out,privileged_perf=False,perf_event='cycles:u')
cases=[(v,b,r,w,c,size) for r in range(1,4) for v in ['before','after'] for b in ['iouring','iouring-poll'] for w,c,size in [(1,32,64),(1,32,4096),(4,128,64),(4,128,4096)]]
random.Random(20261031).shuffle(cases)
for v,b,r,w,c,size in cases:
 args.server=paths[v]
 s=m.sample(args,'control',b,w,c,size,r);s['variant']=v;report['samples'].append(s)
 (out/'aiko.json').write_text(json.dumps(report,indent=2)+'\n');print(len(report['samples']),len(cases),v,b,r,w,c,size,round(s['requests_per_second']),s['server_cpu']['voluntary_switches'],flush=True)
report['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());(out/'aiko.json').write_text(json.dumps(report,indent=2)+'\n')
for w,c,size in [(1,32,64),(1,32,4096),(4,128,64),(4,128,4096)]:
 for b in ['iouring','iouring-poll']:
  vals={v:[s for s in report['samples'] if (s['variant'],s['backend'],s['workers'],s['connections'],s['bytes'])==(v,b,w,c,size)] for v in ['before','after']}
  med={v:statistics.median(s['requests_per_second'] for s in xs) for v,xs in vals.items()}
  print(w,c,size,b,med,'delta',100*(med['after']/med['before']-1),'switches',{v:statistics.median(s['server_cpu']['voluntary_switches'] for s in xs) for v,xs in vals.items()},flush=True)
