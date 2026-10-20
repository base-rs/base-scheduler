from pathlib import Path
import hashlib,importlib.util,json,time,types
base=Path('/home/samuel/.cache/core-scheduler-direct-notification-write-fastpath')
root=Path('/home/samuel/Developer/core-rs/core-scheduler-perf-tokio')
spec=importlib.util.spec_from_file_location('runner',root/'benchmarks/echo/results/readiness-profiling/run.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
out=base/'counted/samples';out.mkdir(exist_ok=True)
paths={'server':base/'counted/benchmark','client':Path('/tmp/core-scheduler-profile-baseline/release/core-scheduler-echo-benchmark')}
r={'purpose':'Mechanism confirmation only; do not compare instrumented throughput to plain timing samples','binary_hashes':{k:hashlib.sha256(p.read_bytes()).hexdigest() for k,p in paths.items()},'instrumentation_sha256':hashlib.sha256((base/'counted/instrumentation.patch').read_bytes()).hexdigest(),'started_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'samples':[]}
a=types.SimpleNamespace(server=paths['server'],client=paths['client'],output=out,privileged_perf=False,perf_event='cycles:u')
for backend,w,c,size in [('iouring',1,1,64),('iouring-poll',1,1,64),('iouring',4,128,64),('iouring-poll',4,128,64)]:
 x=m.sample(a,'counted',backend,w,c,size,1)
 lines=(out/(x['identifier']+'.stderr')).read_text().splitlines()
 rows=[json.loads(line.split('NOTIFICATION_COUNTERS ',1)[1]) for line in lines if line.startswith('NOTIFICATION_COUNTERS ')]
 if not rows:raise RuntimeError('Missing notification counts: '+x['identifier'])
 total={k:sum(row[k] for row in rows) for k in rows[0]}
 x['counter_threads']=rows;x['counters']=total
 assert total['owner_write_attempt']==0,total
 assert total['owner_write_success']==0,total
 assert total['remote_write_attempt']>0,total
 assert total['remote_write_success']>0,total
 assert total['owner_wake_avoided']>0,total
 r['samples'].append(x);(out/'aiko.json').write_text(json.dumps(r,indent=2)+'\n')
 print(x['identifier'],total,flush=True)
r['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());(out/'aiko.json').write_text(json.dumps(r,indent=2)+'\n')
