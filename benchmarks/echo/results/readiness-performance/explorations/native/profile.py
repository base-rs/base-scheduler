import importlib.util,json,subprocess,types
from pathlib import Path
root=Path('/home/samuel/.cache/core-scheduler-native-next')
spec=importlib.util.spec_from_file_location('runner',root/'sample-runner.py'); m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
args=types.SimpleNamespace(server=Path('/home/samuel/.cache/core-scheduler-owner-wake-final/benchmark'),client=Path('/home/samuel/.cache/core-scheduler-owner-wake-final/client'),output=root/'baseline-profile',privileged_perf=False,perf_event='cycles:u')
results=[]
for backend in ['tokio','iouring-poll']:
 r=m.sample(args,'perf',backend,1,32,64,1);results.append(r);print(json.dumps(r),flush=True)
 data=args.output/(r['identifier']+'.data')
 with (args.output/(r['identifier']+'.self.txt')).open('w') as f: subprocess.run(['perf','report','--stdio','--header','--no-children','-g','none','--show-nr-samples','--sort','symbol','--percent-limit','0.1','-i',str(data)],stdout=f,check=True)
 with (args.output/(r['identifier']+'.callgraph.txt')).open('w') as f: subprocess.run(['perf','report','--stdio','--children','-g','graph,1,caller','--sort','symbol','--percent-limit','1','-i',str(data)],stdout=f,check=True)
(args.output/'results.json').write_text(json.dumps(results,indent=2)+'\n')
