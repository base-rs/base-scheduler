import argparse, datetime, hashlib, importlib.util, itertools, json, os, platform, random, statistics, subprocess
from pathlib import Path
from types import SimpleNamespace
p=argparse.ArgumentParser()
p.add_argument('--variants', nargs='+', required=True, help='label:backend:binary')
p.add_argument('--output', type=Path, required=True)
p.add_argument('--workers', nargs='+', type=int, default=[1])
p.add_argument('--connections', nargs='+', type=int, default=[32])
p.add_argument('--bytes', nargs='+', type=int, default=[64,4096])
p.add_argument('--repeats', type=int, default=5)
p.add_argument('--seconds', type=float, default=3)
p.add_argument('--seed', type=int, default=8739)
a=p.parse_args()
root=Path('/home/samuel/Developer/core-rs/core-scheduler-native-monitor')
spec=importlib.util.spec_from_file_location('echo',root/'benchmarks/echo/run.py')
echo=importlib.util.module_from_spec(spec);spec.loader.exec_module(echo)
client=Path('/home/samuel/Developer/core-rs/core-scheduler-monoio-echo/target/release/core-scheduler-echo-benchmark')
args=SimpleNamespace(server_cpus=[0,1,2,3], client_cpus=[4,5,6,7], client_binary=client, client_workers=4, warmup=1, seconds=a.seconds)
variants=[(label,backend,Path(binary)) for label,backend,binary in (v.split(':',2) for v in a.variants)]
def output(*command): return subprocess.check_output(command,cwd=root,text=True).strip()
report=dict(created_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),hostname=platform.node(),platform=platform.platform(),commit=output('git','rev-parse','HEAD'),patch=output('git','diff'),rustc=output('rustc','--version'),cpu_topology=output('lscpu','-e=CPU,CORE,SOCKET,NODE,CACHE'),load_before=os.getloadavg(),arguments=vars(a)|{'output':str(a.output)},binaries={label:{'path':str(binary),'sha256':hashlib.sha256(binary.read_bytes()).hexdigest()} for label,binary in [('client',client)]+[(v[0],v[2]) for v in variants]},samples=[])
rng=random.Random(a.seed)
a.output.parent.mkdir(parents=True,exist_ok=True)
for repeat in range(a.repeats):
    cases=list(itertools.product(a.workers,a.connections,a.bytes));rng.shuffle(cases)
    for w,c,b in cases:
        order=variants.copy();rng.shuffle(order)
        for label,backend,binary in order:
            row=echo.sample(binary,backend,w,c,b,args)
            report['samples'].append(row|dict(variant=label,repeat=repeat+1))
            a.output.write_text(json.dumps(report,indent=2)+'\n')
            print(f'{len(report["samples"]):3} {label:16} w{w} c{c} b{b}: {row["requests_per_second"]:.0f}',flush=True)
report['load_after']=os.getloadavg();a.output.write_text(json.dumps(report,indent=2)+'\n')
for case in itertools.product(a.workers,a.connections,a.bytes):
    print(case,[(label,round(statistics.median(row['requests_per_second'] for row in report['samples'] if row['variant']==label and (row['workers'],row['connections'],row['bytes'])==case))) for label,_,_ in variants],flush=True)
