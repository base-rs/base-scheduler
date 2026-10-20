import importlib.util,json,random,argparse,statistics,hashlib,platform,os,subprocess,time
from pathlib import Path
root=Path('/home/samuel/Developer/core-rs/core-scheduler-perf-interrupt-counters')
base=Path('/home/samuel/.cache/core-scheduler-interrupt-counters')
spec=importlib.util.spec_from_file_location('runner',root/'benchmarks/echo/results/readiness-profiling/run.py')
r=importlib.util.module_from_spec(spec);spec.loader.exec_module(r)
out=base/'matched';out.mkdir(exist_ok=False)
bins={'prototype-frozen':Path('/tmp/core-perf-socket-candidate'),'prototype-counted':base/'prototype-counted','direct-read-frozen':Path('/tmp/core-perf-idle-fix-read'),'direct-read-counted':base/'direct-read-counted'}
client=Path('/tmp/core-scheduler-profile-baseline/release/core-scheduler-echo-benchmark')
report={'sources':{'prototype-frozen':'4a061fbc4355738f39f5eb6ea1db1520db6a753e','prototype-counted':subprocess.check_output(['git','-C',str(root),'rev-parse','65297d6'],text=True).strip(),'direct-read-frozen':'6bd87198787b489950766ca52f34a6f8589d5ddc','direct-read-counted':subprocess.check_output(['git','-C',str(root),'rev-parse','aa17915'],text=True).strip()},'binaries':{k:{'path':str(v),'sha256':hashlib.sha256(v.read_bytes()).hexdigest()} for k,v in bins.items()},'client':{'path':str(client),'sha256':hashlib.sha256(client.read_bytes()).hexdigest()},'counter_build':'cargo build --release --locked -p core-scheduler-echo-benchmark; CARGO_PROFILE_RELEASE_DEBUG=1; RUSTFLAGS unset','frozen_direct_read_build_note':'Existing frozen experimental executable originally built with RUSTFLAGS=-Cdebuginfo=1; retained exactly as requested.','runner_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'sample_runner_sha256':hashlib.sha256((root/'benchmarks/echo/results/readiness-profiling/run.py').read_bytes()).hexdigest(),'seed':20261103,'hostname':platform.node(),'platform':platform.platform(),'server_cpus':'0','client_cpus':'4-7','client_workers':4,'warmup_seconds':1,'measurement_seconds':3,'load_before':os.getloadavg(),'samples':[]}
a=argparse.Namespace(server=None,client=client,output=out,privileged_perf=False)
cases=[(rep,label) for rep in range(1,4) for label in bins]
random.Random(report['seed']).shuffle(cases)
for rep,label in cases:
 a.server=bins[label];x=r.sample(a,label,'iouring',1,1,64,rep);x['variant']=label
 lines=(out/(x['identifier']+'.stderr')).read_text().splitlines()
 rows=[json.loads(line.split('INTERRUPT_COUNTERS ',1)[1]) for line in lines if line.startswith('INTERRUPT_COUNTERS ')]
 x['counter_threads']=rows
 if label.endswith('counted') and not rows:raise RuntimeError('no counter output for '+label)
 if rows:
  total={k:sum(row.get(k,0) for row in rows) for k in rows[0] if k!='worker'};x['counters']=total;x['lifecycle_payload_cqe_pairs']=total['payload_cqe']/2
 report['samples'].append(x);(out/'aiko.json').write_text(json.dumps(report,indent=2)+'\n');print(label,x['identifier'],round(x['requests_per_second']),x['server_cpu']['voluntary_switches'],x.get('counters'),flush=True)
report['load_after']=os.getloadavg();(out/'aiko.json').write_text(json.dumps(report,indent=2)+'\n')
for label in bins:
 rows=[x for x in report['samples'] if x['variant']==label]
 print(label,'median-rps',round(statistics.median(x['requests_per_second'] for x in rows)),'rps',[round(x['requests_per_second']) for x in rows],'voluntary-csw',[x['server_cpu']['voluntary_switches'] for x in rows],flush=True)
