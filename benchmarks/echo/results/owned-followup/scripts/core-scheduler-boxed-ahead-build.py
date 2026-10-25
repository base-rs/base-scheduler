import fcntl, hashlib, json, os, shutil, subprocess
from pathlib import Path

root = Path('/home/samuel/Developer/core-rs/core-scheduler-lean-owned-read')
cache = Path('/home/samuel/.cache/core-scheduler-owned-dispatch')
env = os.environ | {
    'CARGO_TARGET_DIR': '/home/samuel/.cache/core-scheduler-owned-echo-diagnosis/target',
    'CARGO_PROFILE_RELEASE_OPT_LEVEL': '3', 'CARGO_PROFILE_RELEASE_DEBUG': '1',
    'CARGO_PROFILE_RELEASE_LTO': 'false', 'CARGO_PROFILE_RELEASE_CODEGEN_UNITS': '16',
    'CARGO_INCREMENTAL': '0', 'CARGO_ENCODED_RUSTFLAGS': '', 'RUSTFLAGS': '', 'RUST_TEST_THREADS': '1',
}
with open('/tmp/core-scheduler-perf.lock', 'a') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    assert subprocess.check_output(['git', 'status', '--porcelain'], cwd=root) == b''
    paths = ['crates/iouring/src/io_uring_socket_stream_reader.rs', 'crates/iouring/src/io_uring_selector/socket_read.rs', 'crates/iouring/src/io_uring_selector/socket.rs']
    original = {path: (root / path).read_bytes() for path in paths}
    try:
        subprocess.run(['git', 'apply', '--whitespace=nowarn', str(cache / 'boxed-ahead.patch')], cwd=root, check=True)
        for name, command in [('test', ['cargo', 'test', '--locked', '-p', 'core-scheduler-iouring', '--lib']),
                ('build', ['cargo', 'build', '--release', '--locked', '-p', 'core-scheduler-echo-benchmark'])]:
            log = cache / 'logs' / ('boxed-ahead-' + name + '.log')
            with log.open('w') as output:
                result = subprocess.run(command, cwd=root, env=env, stdout=output, stderr=subprocess.STDOUT)
            if result.returncode:
                raise RuntimeError(log.read_text()[-12000:])
            print(name, 'passed', flush=True)
        binary = cache / 'bin/boxed-ahead'
        shutil.copy2(Path(env['CARGO_TARGET_DIR']) / 'release/core-scheduler-echo-benchmark', binary)
        (cache / 'boxed-ahead-build-manifest.json').write_text(json.dumps({
            'base': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
            'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
            'patch_sha256': hashlib.sha256((cache / 'boxed-ahead.patch').read_bytes()).hexdigest(),
            'environment': {k:v for k,v in env.items() if k.startswith(('CARGO_', 'RUSTFLAGS', 'RUST_TEST'))},
        }, indent=2) + '\n')
        with (cache / 'logs/boxed-ahead-layout.log').open('w') as output:
            result = subprocess.run(['cargo', 'rustc', '--release', '--locked', '-p', 'core-scheduler-echo-benchmark', '--lib', '--', '-Zprint-type-sizes'], cwd=root, env=env | {'RUSTC_BOOTSTRAP': '1'}, stdout=output, stderr=subprocess.STDOUT)
        if result.returncode:
            raise RuntimeError((cache / 'logs/boxed-ahead-layout.log').read_text()[-6000:])
        print('\n'.join(line for line in (cache / 'logs/boxed-ahead-layout.log').read_text().splitlines() if 'server::echo::' in line and 'type:' in line), flush=True)
    finally:
        for path, data in original.items():
            (root / path).write_bytes(data)
    subprocess.run(['git', 'diff', '--exit-code'], cwd=root, check=True)
