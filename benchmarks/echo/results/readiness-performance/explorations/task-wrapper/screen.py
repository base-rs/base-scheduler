from pathlib import Path
import importlib.util,json,random,statistics,time,types,hashlib
root=Path(__file__).resolve().parent
out=root/'rawtask-screen';out.mkdir(exist_ok=False)
helper=Path('/home/samuel/.cache/core-scheduler-owner-wake-final/sample-runner.py')
spec=importlib.util.spec_from_file_location('sample_runner',helper);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
args=types.SimpleNamespace(server=root/'rawtask',client=Path('/home/samuel/.cache/core-scheduler-owner-wake-final/client'),output=out,privileged_perf=False)
backends=['tokio','tokio-adapter','tokio-adapter-poll','tokio-adapter-rawtask','tokio-adapter-rawtask-poll']
cases=[(r,b) for r in range(1,4) for b in backends];random.Random(20261020).shuffle(cases)
report={'source':'427be5d + benchmark rawtask-only diagnostic patch','server_sha256':hashlib.sha256(args.server.read_bytes()).hexdigest(),'seed':20261020,'samples':[]}
for r,b in cases:
 s=m.sample(args,'rawtask',b,1,32,64,r);report['samples'].append(s);(out/'aiko.json').write_text(json.dumps(report,indent=2)+'\n');print(len(report['samples']),len(cases),b,round(s['requests_per_second']),flush=True)
med={b:statistics.median(s['requests_per_second'] for s in report['samples'] if s['backend']==b) for b in backends}
print(json.dumps({b:{'median':v,'pct_tokio':v/med['tokio']*100} for b,v in med.items()},indent=2))
