from pathlib import Path
import hashlib,importlib.util,json,os,platform,random,statistics,time,types
root=Path('/home/samuel/.cache/core-scheduler-owner-wake-final')
out=Path(__file__).resolve().parent
server=root/'benchmark'
client=root/'client'
helper=root/'sample-runner.py'
def fingerprint(path):return {'path':str(path),'sha256':hashlib.sha256(path.read_bytes()).hexdigest()}
spec=importlib.util.spec_from_file_location('sample_runner',helper)
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
report={'production_source_commit':'3af77be0f8b8a9303c8680a7f8aab49b1c3d304c','source_build_provenance':json.loads((root/'provenance.json').read_text()),'same_server_executable_for_all_backends':True,'server':fingerprint(server),'client':fingerprint(client),'sample_runner':fingerprint(helper),'runner':fingerprint(Path(__file__)),'platform':platform.platform(),'hostname':platform.node(),'server_cpus':'0 through workers-1','client_cpus':'4-7','client_workers':4,'warmup_seconds':1,'measurement_seconds':3,'seed':20261020,'lock':'exclusive /tmp/core-scheduler-perf.lock','started_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'samples':[]}
args=types.SimpleNamespace(server=server,client=client,output=out,privileged_perf=False)
workloads=[(1,1,64),(1,32,64),(1,32,4096),(4,128,64),(4,128,4096)]
backends=['tokio','tokio-adapter','tokio-adapter-poll','iouring','iouring-poll']
cases=[(rep,backend,workers,connections,size) for rep in range(1,4) for backend in backends for workers,connections,size in workloads]
random.Random(report['seed']).shuffle(cases)
def save():
 temp=out/'aiko.json.tmp';temp.write_text(json.dumps(report,indent=2)+'\n');temp.replace(out/'aiko.json')
save()
for rep,backend,workers,connections,size in cases:
 sample=m.sample(args,'matched',backend,workers,connections,size,rep)
 with (out/(sample['identifier']+'.json')).open('x') as f:json.dump(sample,f,indent=2);f.write('\n')
 report['samples'].append(sample);save()
 print(len(report['samples']),len(cases),backend,workers,connections,size,rep,round(sample['requests_per_second']),flush=True)
report['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());save()
for workers,connections,size in workloads:
 rows={b:[s for s in report['samples'] if (s['backend'],s['workers'],s['connections'],s['bytes'])==(b,workers,connections,size)] for b in backends}
 med={b:statistics.median(s['requests_per_second'] for s in samples) for b,samples in rows.items()}
 print('RESULT',workers,connections,size,json.dumps({b:{'median_rps':v,'pct_of_tokio':100*v/med['tokio']} for b,v in med.items()}),flush=True)
