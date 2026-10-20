import fcntl,hashlib,json,os,subprocess,time
from pathlib import Path
root=Path.home()/'.cache/core-scheduler-hotpath'
work=Path.home()/'Developer/core-rs/core-scheduler-hotpath'
env=dict(os.environ,CARGO_TARGET_DIR=str(root/'target'))
def run(command,name):
 with (root/(name+'.log')).open('w') as out:
  subprocess.run(command,cwd=work,env=env,stdout=out,stderr=subprocess.STDOUT,check=True)
def fingerprint(path): return {'path':str(path),'sha256':hashlib.sha256(path.read_bytes()).hexdigest()}
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 print('Waiting for build lock',flush=True);fcntl.flock(lock,fcntl.LOCK_EX)
 report={'started_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'compiler':subprocess.check_output(['rustc','-vV'],text=True),'source_revision':subprocess.check_output(['git','rev-parse','HEAD'],cwd=work,text=True).strip(),'profile':'release (default optimized Cargo profile), --locked','patch':fingerprint(root/'single-local.patch'),'binaries':{}}
 for label in ['before','single-local']:
  if label=='single-local': run(['git','apply',str(root/'single-local.patch')],'apply')
  run(['cargo','build','--release','--locked','-p','core-scheduler-echo-benchmark'],'build-'+label)
  output=root/(label+'-echo');output.write_bytes((root/'target/release/core-scheduler-echo-benchmark').read_bytes());output.chmod(0o755)
  report['binaries'][label]=fingerprint(output)
 run(['cargo','test','--locked','-p','core-scheduler-native'],'tests-single-local')
 report['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime())
 (root/'build.json').write_text(json.dumps(report,indent=2)+'\n')
 print(json.dumps(report,indent=2),flush=True)
