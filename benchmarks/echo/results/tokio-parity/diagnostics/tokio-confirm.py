import datetime,hashlib,importlib.util,json,pathlib,platform,random,statistics,subprocess,types
root=pathlib.Path.home()/'Developer/core-rs/core-scheduler-parity-tokio'
spec=importlib.util.spec_from_file_location('runner',root/'benchmarks/echo/run.py');r=importlib.util.module_from_spec(spec);spec.loader.exec_module(r)
cache=pathlib.Path.home()/'.cache/core-scheduler-parity';output=cache/'tokio-policy-confirmation.json'
variants=[('raw',cache/'baseline','tokio'),('conservative',cache/'tokio-conservative','tokio'),('adapter',cache/'baseline','tokio-adapter-monitor'),('sendfast-corrected',cache/'tokio-sendfast-corrected','tokio-adapter-monitor')]
args=types.SimpleNamespace(server_cpus=[0,1,2,3],client_cpus=[4,5,6,7],client_binary=cache/'client',client_workers=4,warmup=1,seconds=3)
report=dict(created_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),seed=20261024,source='1840258bbf9cf10c08fefa26fbff54a747f18d08',platform=platform.platform(),rustc=subprocess.check_output(['rustc','--version'],text=True).strip(),build='cargo build --release --locked -p core-scheduler-echo-benchmark; CARGO_PROFILE_RELEASE_DEBUG=1',binaries={name:dict(path=str(binary),sha256=hashlib.sha256(binary.read_bytes()).hexdigest()) for name,binary,_ in variants},client_sha256=hashlib.sha256(args.client_binary.read_bytes()).hexdigest(),patches={name:hashlib.sha256((cache/('tokio-'+name+'.patch')).read_bytes()).hexdigest() for name in ['conservative','sendfast-corrected']},samples=[])
rng=random.Random(report['seed'])
for repeat in range(1,6):
 jobs=variants.copy();rng.shuffle(jobs)
 for name,binary,backend in jobs:
  row=r.sample(binary,backend,1,32,64,args);row.update(variant=name,repeat=repeat);report['samples'].append(row);output.write_text(json.dumps(report,indent=2)+'\n');print(f'{repeat} {name}: {row["requests_per_second"]:.0f}',flush=True)
summary={name:dict(median=statistics.median(x['requests_per_second'] for x in report['samples'] if x['variant']==name),minimum=min(x['requests_per_second'] for x in report['samples'] if x['variant']==name),maximum=max(x['requests_per_second'] for x in report['samples'] if x['variant']==name)) for name,_,_ in variants}
report['summary']=summary;output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(summary,indent=2),flush=True)
