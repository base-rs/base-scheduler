import fcntl, hashlib, json, os, shutil, subprocess
from pathlib import Path

root = Path('/home/samuel/Developer/core-rs/core-scheduler-owned-wake')
cache = Path('/home/samuel/.cache/core-scheduler-owned-dispatch')
env = os.environ | {
    'CARGO_TARGET_DIR': '/home/samuel/.cache/core-scheduler-owned-echo-diagnosis/target',
    'CARGO_PROFILE_RELEASE_OPT_LEVEL': '3', 'CARGO_PROFILE_RELEASE_DEBUG': '1',
    'CARGO_PROFILE_RELEASE_LTO': 'false', 'CARGO_PROFILE_RELEASE_CODEGEN_UNITS': '16',
    'CARGO_INCREMENTAL': '0', 'CARGO_ENCODED_RUSTFLAGS': '', 'RUSTFLAGS': '', 'RUST_TEST_THREADS': '1',
}
paths = ['crates/iouring/src/operation_state.rs', 'crates/iouring/src/io_uring_selector.rs',
    'crates/iouring/src/io_uring_selector/socket_read.rs', 'crates/iouring/src/io_uring_socket_stream_reader.rs', 'crates/native/src/worker.rs']
with open('/tmp/core-scheduler-perf.lock', 'a') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    assert subprocess.check_output(['git', 'status', '--porcelain'], cwd=root) == b''
    original = {path: (root / path).read_bytes() for path in paths}
    report = {'base': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
        'environment': {k: v for k, v in env.items() if k.startswith(('CARGO_', 'RUSTFLAGS', 'RUST_TEST'))}, 'variants': {}}
    try:
        for name, patches in [('local-single-wake', ['local-single-wake']), ('local-lane-reuse', ['local-single-wake', 'lane-reuse'])]:
            for path, data in original.items():
                (root / path).write_bytes(data)
            for patch in patches:
                subprocess.run(['git', 'apply', '--whitespace=nowarn', str(cache / (patch + '.patch'))], cwd=root, check=True)
            subprocess.run(['cargo', 'fmt', '--all'], cwd=root, check=True)
            (cache / (name + '.patch')).write_bytes(subprocess.check_output(['git', 'diff'], cwd=root))
            for action, command in [('test', ['cargo', 'test', '--locked', '-p', 'core-scheduler-iouring', '-p', 'core-scheduler-native', '--lib']),
                    ('build', ['cargo', 'build', '--release', '--locked', '-p', 'core-scheduler-echo-benchmark'])]:
                log = cache / 'logs' / (action + '-' + name + '.log')
                with log.open('w') as out:
                    result = subprocess.run(command, cwd=root, env=env, stdout=out, stderr=subprocess.STDOUT)
                if result.returncode:
                    raise RuntimeError(log.read_text()[-10000:])
                print(action, name, 'passed', flush=True)
            binary = cache / 'bin' / name
            shutil.copy2(Path(env['CARGO_TARGET_DIR']) / 'release/core-scheduler-echo-benchmark', binary)
            report['variants'][name] = {'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                'patch_sha256': hashlib.sha256((cache / (name + '.patch')).read_bytes()).hexdigest()}
            (cache / 'local-single-wake-build-manifest.json').write_text(json.dumps(report, indent=2) + '\n')
    finally:
        for path, data in original.items():
            (root / path).write_bytes(data)
    subprocess.run(['git', 'diff', '--exit-code'], cwd=root, check=True)
