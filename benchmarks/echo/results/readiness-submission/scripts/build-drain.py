import fcntl,hashlib,json,os,shutil,subprocess,time
from pathlib import Path
home=Path.home();cache=home/'.cache/core-scheduler-readiness-research';main=home/'Developer/core-rs/core-scheduler';repo=home/'Developer/core-rs/core-scheduler-readiness-drain';old=home/'.cache/core-scheduler-perf-followup'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 print('Waiting for combined build lock',flush=True);fcntl.flock(lock,fcntl.LOCK_EX)
 subprocess.run(['git','worktree','add','--detach',str(repo),'8400db7da957dfd7dd84de13f210c2b41a98ece1'],cwd=main,check=True)
 subprocess.run(['git','-c','core.whitespace=blank-at-eol','apply',str(cache/'drain-final.patch')],cwd=repo,check=True)
 env=dict(os.environ,PATH=str(home/'.cargo/bin')+':'+os.environ['PATH'],CARGO_TARGET_DIR=str(old/'driver-target'),CARGO_LLVM_COV_TARGET_DIR=str(cache/'coverage-drain'),LLVM_COV=str(old/'llvm-cov-wrapper.py'),LLVM_PROFDATA='/usr/bin/llvm-profdata')
 report={'source':'8400db7da957dfd7dd84de13f210c2b41a98ece1','baseline_build':json.loads((cache/'build.json').read_text()),'patch_sha256':sha(cache/'drain-final.patch'),'production_patch_sha256':sha(cache/'drain-production.patch'),'binaries':{},'verification':[],'script_sha256':sha(Path(__file__))}
 with (cache/'drain-verification.log').open('w') as log:
  for command in [['cargo','bake','--locked','test:coverage','--package','core-scheduler-iouring','--all-features','true'],['cargo','fmt','--all','--','--check'],['cargo','clippy','--locked','-p','core-scheduler-iouring','--all-targets','--all-features','--','-D','warnings']]:
   result=subprocess.run(command,cwd=repo,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=240);report['verification'].append({'command':command,'exit_code':result.returncode})
   if result.returncode:
    (cache/'drain-build-failed.json').write_text(json.dumps(report,indent=2)+'\n');raise SystemExit(result.returncode)
 command=['cargo','build','--release','--locked','-p','core-scheduler-echo-benchmark']
 with (cache/'drain-build.log').open('w') as log:subprocess.run(command,cwd=repo,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
 binary=cache/'readiness-drain-echo';shutil.copy2(Path(env['CARGO_TARGET_DIR'])/'release/core-scheduler-echo-benchmark',binary)
 report['binaries']['readiness-drain']={'path':str(binary),'sha256':sha(binary),'command':command,'source_diff_sha256':hashlib.sha256(subprocess.check_output(['git','diff','--','crates'],cwd=repo)).hexdigest()}
 report['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());(cache/'drain-build.json').write_text(json.dumps(report,indent=2)+'\n')
 print('Combined build and full io_uring verification passed',flush=True)
