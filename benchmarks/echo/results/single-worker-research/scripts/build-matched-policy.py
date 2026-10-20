#!/usr/bin/env python3
"""Benchmark-only polling readiness control, in a separate checkout."""
import fcntl,hashlib,json,shutil,subprocess
from pathlib import Path
root=Path.home()/'.cache/core-scheduler-single-worker-research'
base=Path.home()/'Developer/core-rs/core-scheduler-single-worker-research'
checkout=Path.home()/'Developer/core-rs/core-scheduler-matched-policy-research'
subprocess.run(['git','worktree','add','--detach',str(checkout),'130cd4a67e526e80ba9a5e7e7ebbe81494bd903b'],cwd=base,check=True)
subprocess.run(['git','apply','-'],cwd=checkout,input=(root/'controls.patch').read_bytes(),check=True)
source=checkout/'benchmarks/echo/src/server.rs'
text=source.read_text()
old='const CONSERVATIVE: bool, const ADAPTER_TASKS: bool>'
assert text.count(old)==1
text=text.replace(old,'const CONSERVATIVE: bool, const ADAPTER_TASKS: bool, const POLLING: bool>')
old='socket.readable().await.unwrap();'
assert text.count(old)==1
text=text.replace(old,'''if POLLING {
                            std::future::poll_fn(|cx| socket.poll_read_ready(cx)).await.unwrap();
                        } else {
                            socket.readable().await.unwrap();
                        }''')
for args in ['false, false','true, false','false, true','true, true']:
 text=text.replace('tokio_policy_server::<'+args+'>','tokio_policy_server::<'+args+', false>')
old='"tokio-policy-native" | "tokio-policy-conservative" | "tokio-policy-native-adapter-task"'
assert text.count(old)==1
text=text.replace(old,'"tokio-policy-poll" | '+old)
old='"tokio-policy-native" => runtime.block_on(tokio_policy_server::<false, false, false>(listener, connections)),'
assert text.count(old)==1
text=text.replace(old,old+'\n                "tokio-policy-poll" => runtime.block_on(tokio_policy_server::<true, false, true>(listener, connections)),')
source.write_text(text)
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 fcntl.flock(lock,fcntl.LOCK_EX)
 with (root/'matched-policy-build.log').open('w') as output:
  subprocess.run(['cargo','build','--release','--locked','-p','core-scheduler-echo-benchmark'],cwd=checkout,stdout=output,stderr=subprocess.STDOUT,check=True)
 binary=root/'matched-policy'
 shutil.copy2(checkout/'target/release/core-scheduler-echo-benchmark',binary)
 patch=subprocess.check_output(['git','diff'],cwd=checkout,text=True)
 (root/'matched-policy.patch').write_text(patch)
 (root/'matched-policy-build.json').write_text(json.dumps({
  'base_source':'130cd4a67e526e80ba9a5e7e7ebbe81494bd903b',
  'patch_sha256':hashlib.sha256(patch.encode()).hexdigest(),
  'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),
  'production_changes':'None. Existing benchmark-only controls plus poll_read_ready/try_read loop.',
  'build_command':'cargo build --release --locked -p core-scheduler-echo-benchmark',
 },indent=2)+'\n')
print('Built',binary,flush=True)
