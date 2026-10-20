#!/usr/bin/env python3
import argparse,fcntl,hashlib,importlib.util,itertools,json,platform,random,statistics,time,types
from pathlib import Path
p=argparse.ArgumentParser()
p.add_argument('--output',required=True,type=Path)
p.add_argument('--repeats',type=int,default=2)
p.add_argument('--case',action='append',required=True,help='label,backend,binary')
p.add_argument('--workload',action='append',default=[],help='workers,connections,bytes')
a=p.parse_args()
a.output.mkdir(parents=True,exist_ok=False)
helper=Path('/home/samuel/.cache/core-scheduler-owner-wake-final/sample-runner.py')
client=Path('/home/samuel/.cache/core-scheduler-owner-wake-final/client')
spec=importlib.util.spec_from_file_location('helper',helper)
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
cases=[x.split(',',2) for x in a.case]
workloads=[tuple(map(int,x.split(','))) for x in (a.workload or ['1,1,64','1,32,64','4,128,64'])]
fingerprint=lambda p:{'path':str(p),'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
report={'cases':[{ 'label':label,'backend':backend,'binary':fingerprint(Path(binary))} for label,backend,binary in cases], 'helper':fingerprint(helper),'client':fingerprint(client),'runner':fingerprint(Path(__file__)),'repeats':a.repeats,'workloads':workloads,'warmup_seconds':1,'measurement_seconds':3,'server_cpus':'0 through workers-1','client_cpus':'4-7','platform':platform.platform(),'samples':[]}
args=types.SimpleNamespace(client=client,output=a.output,privileged_perf=False)
runs=list(itertools.product(range(1,a.repeats+1),cases,workloads));random.Random(20261021).shuffle(runs)
def save():
 f=a.output/'aiko.json.tmp';f.write_text(json.dumps(report,indent=2)+'\n');f.replace(a.output/'aiko.json')
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 print('Waiting for exclusive benchmark lock.',flush=True);fcntl.flock(lock,fcntl.LOCK_EX)
 report['started_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());save()
 for repeat,(label,backend,binary),(workers,connections,size) in runs:
  args.server=Path(binary)
  s=m.sample(args,label,backend,workers,connections,size,repeat);s['variant']=label
  (a.output/(s['identifier']+'.json')).write_text(json.dumps(s,indent=2)+'\n')
  report['samples'].append(s);save()
  print(len(report['samples']),len(runs),label,backend,workers,connections,size,repeat,round(s['requests_per_second']),flush=True)
 report['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());save()
for w,c,b in workloads:
 print('RESULT',w,c,b,flush=True)
 for label,backend,binary in cases:
  samples=[s['requests_per_second'] for s in report['samples'] if (s['variant'],s['backend'],s['workers'],s['connections'],s['bytes'])==(label,backend,w,c,b)]
  print(label,backend,'median',round(statistics.median(samples)),'range',round(min(samples)),round(max(samples)),flush=True)
