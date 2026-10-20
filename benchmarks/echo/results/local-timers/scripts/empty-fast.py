import difflib,fcntl,hashlib,json,shutil,subprocess,time
from pathlib import Path
root=Path.home()/'.cache/core-scheduler-local-timers-final';checkout=Path.home()/'Developer/core-rs/core-scheduler-local-timers'
for _ in range(600):
 p=root/'timer-stat/results.json'
 if p.exists() and len(json.loads(p.read_text())['samples'])==8:break
 time.sleep(1)
else:raise RuntimeError('Final measurements did not finish')
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 fcntl.flock(lock,fcntl.LOCK_EX)
 source=checkout/'crates/timers/src/timers.rs';original=source.read_text()
 needle='''        pub fn next_deadline(&mut self) -> Option<Instant> {
            self.0.flush();'''
 assert original.count(needle)==1
 updated=original.replace(needle,'''        pub fn next_deadline(&mut self) -> Option<Instant> {
            if self.0.heap.is_empty() && self.0.scheduled.is_empty() {
                return None;
            }
            self.0.flush();''')
 patch=''.join(difflib.unified_diff(original.splitlines(keepends=True),updated.splitlines(keepends=True),fromfile='a/crates/timers/src/timers.rs',tofile='b/crates/timers/src/timers.rs'))
 (root/'empty-fast.patch').write_text(patch);source.write_text(updated)
 try:
  with (root/'empty-fast-build.log').open('w') as output:
   subprocess.run(['cargo','build','--release','--locked','-p','core-scheduler-echo-benchmark','--bin','core-scheduler-echo-benchmark','--features','core-scheduler-native/worker-timers'],cwd=checkout,stdout=output,stderr=subprocess.STDOUT,check=True)
  dest=root/'empty-fast-echo';shutil.copy2(checkout/'target/release/core-scheduler-echo-benchmark',dest)
  with (root/'empty-fast-tests.log').open('w') as output:subprocess.run(['cargo','test','--locked','-p','core-scheduler-native','--all-features','--lib'],cwd=checkout,stdout=output,stderr=subprocess.STDOUT,check=True)
  report={'source':json.loads((root/'build.json').read_text())['source'],'base_patch_sha256':hashlib.sha256((root/'candidate.patch').read_bytes()).hexdigest(),'additional_patch_sha256':hashlib.sha256(patch.encode()).hexdigest(),'binary':{'path':str(dest),'sha256':hashlib.sha256(dest.read_bytes()).hexdigest()}}
  (root/'empty-fast-build.json').write_text(json.dumps(report,indent=2)+'\n')
 finally:source.write_text(original)
 print('Empty check built and tested',flush=True)
command=['python3',str(Path.home()/'.cache/core-scheduler-single-worker-research/run.py'),'--output',str(root/'empty-fast'),'--client',str(Path.home()/'.cache/core-scheduler-owner-wake-final/client'),'--build',str(root/'empty-fast-build.json'),'--repeats','3','--workload','1,1,64']
for variant in ['before','local','empty-fast']:
 binary=root/('empty-fast-echo' if variant=='empty-fast' else variant+'-core-scheduler-echo-benchmark')
 for backend in ['iouring','iouring-poll']+(['tokio'] if variant!='local' else []):command+=['--case',','.join([variant,backend,str(binary)])]
subprocess.run(command,check=True)
