import hashlib,json,os,subprocess
from pathlib import Path
cache=Path('/home/samuel/.cache/core-scheduler-next-final')
def run(*args):return subprocess.check_output(args,text=True).strip()
def fingerprint(path):
 p=Path(path)
 return {'path':str(p),'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
report={
 'before_source':'427be5d2ef6c610a5d5de5b10570c938b65d6c2d',
 'after_source':'958ac6cf9862fdce2fc8de18c8864c4f971b6abc',
 'after_checkout':'Before source plus archived source.patch, matching the after source commit; canonical user checkout left untouched.',
 'build_command':'CARGO_TARGET_DIR=/home/samuel/.cache/core-scheduler-next-final/target cargo build --release --locked -p core-scheduler-echo-benchmark',
 'profile':'Default Cargo release (optimized); no Cargo.toml profile overrides, explicit LTO, target-cpu overrides, or dependency changes.',
 'build_environment':{key:os.environ.get(key) for key in ['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','CARGO_PROFILE_RELEASE_OPT_LEVEL','CARGO_PROFILE_RELEASE_LTO','CARGO_PROFILE_RELEASE_CODEGEN_UNITS']},
 'rustc':run('rustc','-Vv'),'cargo':run('cargo','-V'),'kernel':run('uname','-a'),
 'cpu':run('lscpu'),'topology':run('lscpu','-e=CPU,CORE,SOCKET,NODE,CACHE'),
 'frequency_policy':{str(p):p.read_text().strip() for p in Path('/sys/devices/system/cpu/cpufreq').glob('policy*/scaling_governor')},
 'exclusive_timing_lock':'/tmp/core-scheduler-perf.lock; builds and tests use shared lock.',
 'source_patch':fingerprint('/tmp/core-scheduler-next-final.patch'),
 'cargo_lock':fingerprint('Cargo.lock'),
 'binaries':{name:fingerprint(cache/name) for name in ['before','after']},
 'client':fingerprint('/home/samuel/.cache/core-scheduler-owner-wake-final/client'),
 'runner':fingerprint(cache/'run.py'),'helper':fingerprint(cache/'sample-runner.py'),
 'server_affinity':'CPU0 for one worker; CPUs0–3 for four workers, separate physical cores in one last-level cache.',
 'client_affinity':'CPUs4–7; four workers, separate physical cores sharing the server last-level cache.',
 'matrix':'Five workloads x five backends x before/after x three repeats; 150 complete samples. 1s warmup, 3s measurement; fixed-seed randomized order.',
 'scope':'Closed-loop persistent IPv4 loopback TCP echo; one outstanding request per connection, TCP_NODELAY, 64KiB server buffers. Payload validated by fixed client. Setup/warmup excluded from throughput and RTT, included in lifecycle CPU.'
}
report['release_fingerprints'] = [json.loads(path.read_text()) for path in (cache/'target/release/.fingerprint').glob('core-scheduler-echo-benchmark-*/bin-core-scheduler-echo-benchmark.json')]
(cache/'final'/'provenance.json').write_text(json.dumps(report,indent=2)+'\n')
