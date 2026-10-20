from pathlib import Path
import argparse, hashlib, importlib.util, json, os, platform, random, shutil, statistics, subprocess, time
base=Path('/home/samuel/.cache/core-scheduler-self-wake')
root=Path('/home/samuel/Developer/core-rs/core-scheduler-perf-interrupt-counters')
base.joinpath('matched').mkdir(exist_ok=False)
base.joinpath('binaries').mkdir(exist_ok=False)
shutil.copy2(root/'benchmarks/echo/results/readiness-profiling/run.py',base/'sample-runner.py')
spec=importlib.util.spec_from_file_location('sample_runner',base/'sample-runner.py');r=importlib.util.module_from_spec(spec);spec.loader.exec_module(r)
originals={'prototype-frozen':Path('/tmp/core-perf-socket-candidate'),'prototype-frozen-counted':Path('/home/samuel/.cache/core-scheduler-interrupt-counters/prototype-counted'),'selected-frozen':Path('/tmp/core-perf-socket-selected')}
for label in ['prototype-fixed','prototype-fixed-counted','selected-fixed','selected-fixed-counted','selected-frozen-counted']:originals[label]=base/label
bins={}
for label,source in originals.items():
 dst=base/'binaries'/label;shutil.copy2(source,dst);bins[label]=dst
client=base/'binaries/client';shutil.copy2('/tmp/core-scheduler-profile-baseline/release/core-scheduler-echo-benchmark',client)
def fingerprint(p):return {'path':str(p),'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
sources=json.loads((base/'sources.json').read_text());sources['prototype-frozen-counted']='65297d6469f1df23cc08ef1a94f6a11ad6fa29ed'
report={'sources':sources,'frozen_selected_build_source':'938e7e641f8d186991a0f46f211f4ce3d19c929e','binaries':{label:fingerprint(path) for label,path in bins.items()},'client':fingerprint(client),'build':'cargo build --release --locked -p core-scheduler-echo-benchmark; CARGO_PROFILE_RELEASE_DEBUG=1; RUSTFLAGS unset','seed':20261020,'hostname':platform.node(),'platform':platform.platform(),'server_cpus':'0','client_cpus':'4-7','client_workers':4,'warmup_seconds':1,'measurement_seconds':3,'load_before':os.getloadavg(),'samples':[]}
# Freeze provenance before samples start.
(base/'provenance.json').write_text(json.dumps(report,indent=2)+'\n')
a=argparse.Namespace(server=None,client=client,output=base/'matched',privileged_perf=False)
cases=[(rep,label) for rep in range(1,4) for label in bins];random.Random(report['seed']).shuffle(cases)
for rep,label in cases:
 a.server=bins[label];x=r.sample(a,label,'iouring',1,1,64,rep);x['variant']=label
 lines=(a.output/(x['identifier']+'.stderr')).read_text().splitlines()
 rows=[json.loads(line.split('INTERRUPT_COUNTERS ',1)[1]) for line in lines if line.startswith('INTERRUPT_COUNTERS ')];x['counter_threads']=rows
 if label.endswith('counted') and not rows:raise RuntimeError('missing counters '+label)
 if rows:
  total={k:sum(row.get(k,0) for row in rows) for k in rows[0] if k!='worker'};x['counters']=total;x['lifecycle_payload_cqe_pairs']=total['payload_cqe']/2
 report['samples'].append(x);(a.output/'aiko.json').write_text(json.dumps(report,indent=2)+'\n')
 print(label,x['identifier'],round(x['requests_per_second']),x['server_cpu']['voluntary_switches'],x.get('counters'),flush=True)
report['load_after']=os.getloadavg();(a.output/'aiko.json').write_text(json.dumps(report,indent=2)+'\n')
for label in bins:
 rows=[x for x in report['samples'] if x['variant']==label]
 print(label,'median-rps',round(statistics.median(x['requests_per_second'] for x in rows)),'rps',[round(x['requests_per_second']) for x in rows],'voluntary-csw',[x['server_cpu']['voluntary_switches'] for x in rows],flush=True)
