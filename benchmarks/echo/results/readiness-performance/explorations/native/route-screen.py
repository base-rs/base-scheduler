import importlib.util,types,json,hashlib,subprocess,random
from pathlib import Path
root=Path('/home/samuel/.cache/core-scheduler-native-next')
spec=importlib.util.spec_from_file_location('runner',root/'sample-runner.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
output=root/'route-screen';output.mkdir(exist_ok=False)
base=Path('/home/samuel/.cache/core-scheduler-owner-wake-final/benchmark');candidate=root/'route'
client=Path('/home/samuel/.cache/core-scheduler-owner-wake-final/client')
args=types.SimpleNamespace(client=client,output=output,privileged_perf=False,perf_event='cycles:u')
source=Path('/home/samuel/Developer/core-rs/core-scheduler-perf-native-next')
(output/'source.patch').write_bytes(subprocess.check_output(['git','-C',str(source),'diff']))
report={'binaries':{name:{'path':str(path),'sha256':hashlib.sha256(path.read_bytes()).hexdigest()} for name,path in [('baseline',base),('candidate',candidate),('client',client)]},'source':subprocess.check_output(['git','-C',str(source),'rev-parse','HEAD'],text=True).strip(),'samples':[],'seed':20261021}
cases=[(kind,w,c,r) for r in range(1,4) for w,c in [(1,1),(1,32)] for kind in ['baseline','candidate']];random.Random(report['seed']).shuffle(cases)
for kind,w,c,r in cases:
 args.server=base if kind=='baseline' else candidate
 result=m.sample(args,kind,'iouring-poll',w,c,64,r);result['kind']=kind;report['samples'].append(result)
 (output/'results.json').write_text(json.dumps(report,indent=2)+'\n');print(kind,w,c,r,result['requests_per_second'],flush=True)
