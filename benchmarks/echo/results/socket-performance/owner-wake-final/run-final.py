from pathlib import Path
import hashlib,importlib.util,json,os,platform,random,runpy,statistics,subprocess,time,types
base=Path('/home/samuel/.cache/core-scheduler-owner-wake-final')
root=Path('/home/samuel/Developer/core-rs/core-scheduler-perf-tokio')
paths={k:base/k for k in ['control','benchmark','client']}
def fingerprint(p):return {'path':str(p),'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
def cmd(*a):return subprocess.check_output(a,cwd=root,text=True).strip()
provenance={'source_base':cmd('git','rev-parse','HEAD'),'source_patch':fingerprint(base/'source.patch'),'binaries':{k:fingerprint(p) for k,p in paths.items()},'runner':fingerprint(base/'run-final.py'),'sample_runner':fingerprint(base/'sample-runner.py'),'build_command':'cargo build --release --locked -p core-scheduler-echo-benchmark','build_target_dir':'/home/samuel/.cache/core-scheduler-perf-tokio','release_manifest':fingerprint(base/'Cargo.toml.build'),'build_overrides':{k:os.environ.get(k) for k in ['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','CARGO_PROFILE_RELEASE_OPT_LEVEL','CARGO_PROFILE_RELEASE_LTO','CARGO_PROFILE_RELEASE_CODEGEN_UNITS','CARGO_PROFILE_RELEASE_DEBUG']},'rustc':cmd('rustc','-Vv'),'cargo':cmd('cargo','-V'),'platform':platform.platform(),'hostname':platform.node(),'cpu':cmd('lscpu'),'server_cpus':'0 through workers-1','client_cpus':'4-7','client_workers':4,'warmup_seconds':1,'measurement_seconds':3,'lock':'exclusive /tmp/core-scheduler-perf.lock','started_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime())}
(base/'provenance.json').write_text(json.dumps(provenance,indent=2)+'\n')
runpy.run_path(str(base/'idle.py'))
idle=json.loads((base/'idle/idle.json').read_text())
assert all(sum(x['cpu_seconds'].values())<=.02 for x in idle['samples'] if x['variant']=='after'),idle
spec=importlib.util.spec_from_file_location('runner',base/'sample-runner.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
for phase,workloads in [('concurrent',[(1,32,64),(1,32,4096),(4,128,64),(4,128,4096)]),('serial64',[(1,1,64)])]:
 out=base/phase;out.mkdir(exist_ok=False)
 report={'phase':phase,'provenance':provenance,'samples':[]}
 (out/'aiko.json').write_text(json.dumps(report,indent=2)+'\n')
 args=types.SimpleNamespace(client=paths['client'],output=out,privileged_perf=False,perf_event='cycles:u')
 cases=[(v,b,r,w,c,size) for r in range(1,4) for v in ['before','after'] for b in ['iouring','iouring-poll'] for w,c,size in workloads]
 random.Random(20261102 if phase=='concurrent' else 20261103).shuffle(cases)
 for v,b,r,w,c,size in cases:
  args.server=paths['control' if v=='before' else 'benchmark']
  x=m.sample(args,v,b,w,c,size,r);x['variant']=v
  with (out/(x['identifier']+'.json')).open('x') as f:json.dump(x,f,indent=2);f.write('\n')
  report['samples'].append(x)
  (out/'aiko.json').write_text(json.dumps(report,indent=2)+'\n')
  print(phase,len(report['samples']),len(cases),v,b,r,w,c,size,round(x['requests_per_second']),x['server_cpu']['voluntary_switches'],flush=True)
 report['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());(out/'aiko.json').write_text(json.dumps(report,indent=2)+'\n')
 for w,c,size in workloads:
  for b in ['iouring','iouring-poll']:
   rows={v:[x for x in report['samples'] if (x['variant'],x['backend'],x['workers'],x['connections'],x['bytes'])==(v,b,w,c,size)] for v in ['before','after']}
   med={v:statistics.median(x['requests_per_second'] for x in xs) for v,xs in rows.items()}
   print(phase,w,c,size,b,med,'delta',100*(med['after']/med['before']-1),flush=True)
provenance['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());(base/'provenance.json').write_text(json.dumps(provenance,indent=2)+'\n')
