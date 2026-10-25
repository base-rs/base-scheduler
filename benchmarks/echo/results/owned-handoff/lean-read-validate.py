import fcntl, hashlib, json, os, shutil, subprocess
from pathlib import Path

parent = Path('/home/samuel/Developer/core-rs/core-scheduler-owned-dispatch')
root = Path('/home/samuel/Developer/core-rs/core-scheduler-lean-owned-read')
cache = Path('/home/samuel/.cache/core-scheduler-owned-dispatch')
tooling = '/home/samuel/.cache/core-scheduler-owned-receive-pr/tools/bin'
env = os.environ | {
    'PATH': tooling + ':' + os.environ['PATH'],
    'CARGO_TARGET_DIR': '/home/samuel/.cache/core-scheduler-owned-echo-diagnosis/target',
    'CARGO_LLVM_COV_TARGET_DIR': str(cache / 'coverage-lean-read'),
    'LLVM_COV': tooling + '/llvm-cov', 'LLVM_PROFDATA': '/usr/bin/llvm-profdata',
    'CARGO_PROFILE_RELEASE_OPT_LEVEL': '3', 'CARGO_PROFILE_RELEASE_DEBUG': '1',
    'CARGO_PROFILE_RELEASE_LTO': 'false', 'CARGO_PROFILE_RELEASE_CODEGEN_UNITS': '16',
    'CARGO_INCREMENTAL': '0', 'CARGO_ENCODED_RUSTFLAGS': '', 'RUSTFLAGS': '', 'RUST_TEST_THREADS': '1',
}
with open('/tmp/core-scheduler-perf.lock', 'a') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    subprocess.run(['git', 'fetch', '/home/samuel/.cache/core-scheduler-lean-owned-read.bundle', 'perf/lean-owned-read'], cwd=parent, check=True)
    commit = subprocess.check_output(['git', 'rev-parse', 'FETCH_HEAD'], cwd=parent, text=True).strip()
    if not root.exists():
        subprocess.run(['git', 'worktree', 'add', '--detach', str(root), commit], cwd=parent, check=True)
    assert subprocess.check_output(['git', 'status', '--porcelain'], cwd=root) == b''
    assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip() == commit
    for name, command in [
        ('context', ['cargo', 'bake', '--locked', 'agent:context:install']),
        ('fmt', ['cargo', 'fmt', '--all', '--', '--check']),
        ('clippy', ['cargo', 'clippy', '--workspace', '--all-targets', '--all-features', '--locked', '--', '-D', 'warnings']),
        ('docs', ['cargo', 'doc', '--workspace', '--all-features', '--no-deps', '--locked']),
        ('coverage', ['cargo', 'bake', '--locked', 'test:coverage', '--all-targets', 'true', '--all-features', 'true']),
        ('build', ['cargo', 'build', '--release', '--locked', '-p', 'core-scheduler-echo-benchmark']),
    ]:
        log = cache / 'logs' / ('lean-final-' + name + '.log')
        with log.open('w') as output:
            result = subprocess.run(command, cwd=root, env=env | {'RUSTDOCFLAGS': '-D warnings'}, stdout=output, stderr=subprocess.STDOUT)
        if result.returncode:
            raise RuntimeError(log.read_text()[-12000:])
        print(name, 'passed', flush=True)
    binary = cache / 'bin/lean-final'
    shutil.copy2(Path(env['CARGO_TARGET_DIR']) / 'release/core-scheduler-echo-benchmark', binary)
    report = {'commit': commit, 'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
        'rustc': subprocess.check_output(['rustc', '-Vv'], text=True),
        'environment': {k:v for k,v in env.items() if k.startswith(('CARGO_', 'RUSTFLAGS', 'RUST_TEST', 'LLVM_'))}}
    (cache / 'lean-final-build-manifest.json').write_text(json.dumps(report, indent=2) + '\n')
