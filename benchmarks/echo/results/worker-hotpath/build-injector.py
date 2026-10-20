import fcntl,hashlib,json,os,subprocess,time
from pathlib import Path
root=Path.home()/'.cache/core-scheduler-hotpath'
work=Path.home()/'Developer/core-rs/core-scheduler-hotpath-injector'
env=dict(os.environ,CARGO_TARGET_DIR=str(root/'target'))
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 print('Waiting for injector build lock',flush=True);fcntl.flock(lock,fcntl.LOCK_EX)
 if not work.exists(): subprocess.run(['git','worktree','add','--detach',str(work),'0d2c4f1'],cwd=Path.home()/'Developer/core-rs/core-scheduler',check=True)
 subprocess.run(['git','apply',str(root/'injector-empty.patch')],cwd=work,check=True)
 with (root/'build-injector-empty.log').open('w') as out: subprocess.run(['cargo','build','--release','--locked','-p','core-scheduler-echo-benchmark'],cwd=work,env=env,stdout=out,stderr=subprocess.STDOUT,check=True)
 output=root/'injector-empty-echo';output.write_bytes((root/'target/release/core-scheduler-echo-benchmark').read_bytes());output.chmod(0o755)
 report=json.loads((root/'build.json').read_text());report['variant']='injector-empty';report['variant_patch_sha256']=hashlib.sha256((root/'injector-empty.patch').read_bytes()).hexdigest();report['variant_binary_sha256']=hashlib.sha256(output.read_bytes()).hexdigest();report['variant_source_path']=str(work);report['variant_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());report['compiler_profile_environment']={k:v for k,v in os.environ.items() if k in ['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTC_WRAPPER','CARGO_BUILD_TARGET'] or k.startswith('CARGO_PROFILE_')}
 with (root/'tests-injector-empty.log').open('w') as out: subprocess.run(['cargo','test','--locked','-p','core-scheduler-native'],cwd=work,env=env,stdout=out,stderr=subprocess.STDOUT,check=True)
 (root/'build-injector-empty.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2),flush=True)
