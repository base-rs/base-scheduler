from pathlib import Path
import subprocess, shutil, os, json
root=Path('/home/samuel/Developer/core-rs/core-scheduler-perf-interrupt-counters')
base=Path('/home/samuel/.cache/core-scheduler-self-wake')
env=os.environ.copy();env.pop('RUSTFLAGS',None);env['CARGO_PROFILE_RELEASE_DEBUG']='1';env['CARGO_TARGET_DIR']='/home/samuel/.cache/core-scheduler-interrupt-counters/build'
def git(*a):return subprocess.check_output(['git','-C',str(root),*a],text=True).strip()
def commit(message):
 git('add','crates');git('-c','commit.gpgsign=false','commit','-m',message);return git('rev-parse','HEAD')
def build(label):
 with (base/('build-'+label+'.log')).open('w') as log:
  subprocess.run(['flock','-s','/tmp/core-scheduler-perf.lock','cargo','build','--release','--locked','-p','core-scheduler-echo-benchmark'],cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
 shutil.copy2(Path(env['CARGO_TARGET_DIR'])/'release/core-scheduler-echo-benchmark',base/label)
 print(label,git('rev-parse','HEAD'),flush=True)
sources={'prototype-fixed':git('rev-parse','HEAD'),'prototype-frozen':'4a061fbc4355738f39f5eb6ea1db1520db6a753e','selected-frozen':git('rev-parse','63da755')}
# Add the prior lightweight prototype counters separately from the runtime fix.
git('apply','/home/samuel/.cache/core-scheduler-interrupt-counters/patches/prototype.patch')
sources['prototype-fixed-counted']=commit('Count prototype activity after avoiding same-thread interrupts.')
(base/'patches/prototype-fixed-counters.patch').write_text(git('diff',sources['prototype-fixed'],'HEAD')+'\n')
build('prototype-fixed-counted')
# Apply the exact same runtime change to the selected implementation.
git('switch','-c','perf/self-wake-selected','63da755')
git('apply',str(base/'patches/self-wake.patch'))
sources['selected-fixed']=commit('Avoid interrupting a worker from its own thread.')
build('selected-fixed')
# The selected implementation has no interrupt SQE/CQE; retain common counters.
p=root/'crates/iouring/src/interrupt_counters.rs'
p.write_text(git('show','65297d6:crates/iouring/src/interrupt_counters.rs')+'\n')
p=root/'crates/iouring/src/io_uring_selector.rs';s=p.read_text()
needle='const CANCEL_TAG: u64 = 1 << 63;';assert s.count(needle)==1
s=s.replace(needle,'#[path = "interrupt_counters.rs"]\npub(crate) mod interrupt_counters;\nuse interrupt_counters as counters;\n\n'+needle)
a='        let _ = name;\n        operation()';assert s.count(a)==1
s=s.replace(a,'        let result = operation();\n        counters::syscall(name, &result);\n        result')
a='    pub(crate) fn turn(&mut self, timeout: Option<Duration>) -> io::Result<bool> {\n';assert s.count(a)==1
s=s.replace(a,a+'        counters::increment(counters::TURN);\n')
a='            // Every untagged CQE is the single original completion of a\n';assert s.count(a)==1
s=s.replace(a,'            counters::increment(counters::PAYLOAD_CQE);\n'+a)
p.write_text(s)
p=root/'crates/iouring/src/io_uring_worker.rs';s=p.read_text()
a='        Self { driver, route }\n';assert s.count(a)==1
s=s.replace(a,'        crate::io_uring_selector::interrupt_counters::worker(true);\n'+a)
a='        self.driver.stop();\n';assert s.count(a)==1
p.write_text(s.replace(a,a+'        crate::io_uring_selector::interrupt_counters::worker(false);\n'))
sources['selected-fixed-counted']=commit('Count selected activity after avoiding same-thread interrupts.')
(base/'patches/selected-fixed-counters.patch').write_text(git('diff',sources['selected-fixed'],'HEAD')+'\n')
build('selected-fixed-counted')
(base/'sources.json').write_text(json.dumps(sources,indent=2)+'\n')
print(json.dumps(sources,indent=2),flush=True)
