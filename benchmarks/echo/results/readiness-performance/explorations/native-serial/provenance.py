from pathlib import Path
import json,hashlib,os,subprocess,difflib
root=Path('/home/samuel/.cache/core-scheduler-native-serial-next')
source=Path('/home/samuel/Developer/core-rs/core-scheduler-native-serial-next')
p=source/'crates/iouring/src/io_uring_selector.rs';candidate=p.read_text()
needle='''                IoUring::builder()
                    .build(self.ring.params().sq_entries())'''
assert candidate.count(needle)==1
selected=candidate.replace(needle,'''                IoUring::builder()
                    .setup_coop_taskrun()
                    .setup_taskrun_flag()
                    .build(self.ring.params().sq_entries())''')
(root/'disabled-flags.patch').write_text(''.join(difflib.unified_diff(selected.splitlines(True),candidate.splitlines(True),fromfile='selected/crates/iouring/src/io_uring_selector.rs',tofile='no-coop/crates/iouring/src/io_uring_selector.rs')))
fingerprint=lambda p:{'path':str(p),'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
run=lambda *args:subprocess.check_output(args,text=True).strip()
report={'base':run('git','-C',str(source),'rev-parse','HEAD'),'selected_source':'958ac6cf9862fdce2fc8de18c8864c4f971b6abc','candidate_delta':'Remove only setup_coop_taskrun and setup_taskrun_flag from the replacement-ring builder; retain started tracking and original replacement lifecycle.','source_patch':fingerprint(root/'selected-source.patch'),'candidate_patch':fingerprint(root/'no-coop-source.patch'),'disabled_flags_patch':fingerprint(root/'disabled-flags.patch'),'candidate_selector_source_sha256':hashlib.sha256(candidate.encode()).hexdigest(),'selected_selector_source_sha256':hashlib.sha256(selected.encode()).hexdigest(),'binaries':{key:fingerprint(Path(path)) for key,path in [('before','/home/samuel/.cache/core-scheduler-next-final/before'),('selected','/home/samuel/.cache/core-scheduler-next-final/after'),('no-coop',str(root/'no-coop'))]},'runner':fingerprint(root/'run.py'),'helper':fingerprint(root/'sample-runner.py'),'client':fingerprint(Path('/home/samuel/.cache/core-scheduler-owner-wake-final/client')),'cargo_lock':fingerprint(source/'Cargo.lock'),'build_command':'CARGO_TARGET_DIR=/home/samuel/.cache/core-scheduler-native-serial-next/target cargo build --release --locked -p core-scheduler-echo-benchmark','profile':'Default optimized Cargo release; no release-profile, RUSTFLAGS, LTO, target-cpu, or dependency changes.','build_environment':{k:os.environ.get(k) for k in ['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','CARGO_PROFILE_RELEASE_DEBUG','CARGO_PROFILE_RELEASE_OPT_LEVEL','CARGO_PROFILE_RELEASE_LTO','CARGO_PROFILE_RELEASE_CODEGEN_UNITS']},'rustc':run('rustc','-Vv'),'cargo':run('cargo','-V'),'machine':run('uname','-a'),'topology':run('lscpu','-e=CPU,CORE,SOCKET,NODE,CACHE'),'timing_lock':'Exclusive /tmp/core-scheduler-perf.lock; builds use shared mode.','scope':'Borrowed iouring-poll server, one worker, one TCP connection,64-byte validated payload; five randomized repetitions for each binary,1s warmup3s measurement; serverCPU0,clientCPUs4-7,four client workers.'}
(root/'provenance.json').write_text(json.dumps(report,indent=2)+'\n')
