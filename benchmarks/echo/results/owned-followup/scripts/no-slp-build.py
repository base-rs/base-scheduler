import fcntl, hashlib, json, os, shutil, subprocess
from pathlib import Path

root = Path('/home/samuel/Developer/core-rs/core-scheduler-lean-owned-read')
cache = Path('/home/samuel/.cache/core-scheduler-owned-dispatch')
env = os.environ | {
    'CARGO_TARGET_DIR': '/home/samuel/.cache/core-scheduler-owned-dispatch/no-slp-target',
    'CARGO_PROFILE_RELEASE_OPT_LEVEL': '3', 'CARGO_PROFILE_RELEASE_DEBUG': '1',
    'CARGO_PROFILE_RELEASE_LTO': 'false', 'CARGO_PROFILE_RELEASE_CODEGEN_UNITS': '16',
    'CARGO_INCREMENTAL': '0', 'CARGO_ENCODED_RUSTFLAGS': '-C\x1fno-vectorize-slp', 'RUSTFLAGS': '-C no-vectorize-slp', 'RUST_TEST_THREADS': '1',
}
with open('/tmp/core-scheduler-perf.lock', 'a') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    assert subprocess.check_output(['git', 'status', '--porcelain'], cwd=root) == b''
    for name, command in [('build', ['cargo', 'build', '--release', '--locked', '-p', 'core-scheduler-echo-benchmark'])]:
        log = cache / 'logs' / ('no-slp-' + name + '.log')
        with log.open('w') as output:
            result = subprocess.run(command, cwd=root, env=env, stdout=output, stderr=subprocess.STDOUT)
        if result.returncode:
            raise RuntimeError(log.read_text()[-12000:])
        print(name, 'passed', flush=True)
    binary = cache / 'bin/no-slp'
    shutil.copy2(Path(env['CARGO_TARGET_DIR']) / 'release/core-scheduler-echo-benchmark', binary)
    (cache / 'no-slp-build-manifest.json').write_text(json.dumps({
        'base': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
        'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
        'environment': {k:v for k,v in env.items() if k.startswith(('CARGO_', 'RUSTFLAGS', 'RUST_TEST'))},
    }, indent=2) + '\n')
    subprocess.run(['git', 'diff', '--exit-code'], cwd=root, check=True)
