import fcntl,hashlib,json,os,platform,subprocess,time
from pathlib import Path
root=Path.home()/'.cache/core-scheduler-local-timers-final';checkout=Path.home()/'Developer/core-rs/core-scheduler-local-timers'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 fcntl.flock(lock,fcntl.LOCK_EX)
 patch=subprocess.check_output(['git','diff'],cwd=checkout)+subprocess.run(['git','diff','--no-index','/dev/null','crates/native/src/worker_timers.rs'],cwd=checkout,capture_output=True).stdout
 assert patch==(root/'candidate.patch').read_bytes()
 report={'collected_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'hostname':platform.node(),'platform':platform.platform(),'cpu_topology':subprocess.check_output(['lscpu','-e=CPU,CORE,SOCKET,NODE,CACHE'],text=True),'source_patch_verified_sha256':hashlib.sha256(patch).hexdigest(),'harness_sha256':sha(checkout/'benchmarks/echo/src/bin/timers.rs'),'mixed_server_sha256':sha(root/'mixed-server.rs'),'compiler_environment_overrides_present':[k for k in os.environ if k in ['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS'] or k.startswith('CARGO_PROFILE_RELEASE_')],'binaries':{}}
 for variant in ['before','local']:
  report['binaries'][variant]={name:sha(root/(variant+'-'+name)) for name in ['timers','core-scheduler-echo-benchmark']}
 (root/'provenance.json').write_text(json.dumps(report,indent=2)+'\n')
 print('Source patch and harness provenance verified',flush=True)
