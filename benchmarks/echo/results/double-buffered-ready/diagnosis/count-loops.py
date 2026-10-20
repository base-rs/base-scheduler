#!/usr/bin/env python3
"""Build temporary counters; these binaries are for event counts, not performance comparisons."""
import fcntl
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

checkout, output = map(Path, sys.argv[1:3])
output.mkdir(parents=True, exist_ok=True)
def replace(source, old, new):
    assert source.count(old) == 1, old
    return source.replace(old, new)

builds = {}
with open('/tmp/core-scheduler-perf.lock', 'a') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    for label, commit in [('before', 'ddc994d'), ('after', 'b03e582'), ('skip-self-wake', 'b03e582')]:
        if sys.argv[3:] and label not in sys.argv[3:]:
            continue
        subprocess.run(['git', 'checkout', '--detach', commit], cwd=checkout, check=True)
        worker = checkout / 'crates/native/src/worker.rs'
        source = worker.read_text()
        source = replace(source, 'thread_local! {', '''thread_local! {
    static DIAGNOSTICS: [std::cell::Cell<usize>; 8] = const { [const { std::cell::Cell::new(0) }; 8] };''')
        source += '''
fn count(index: usize) {
    DIAGNOSTICS.with(|counts| counts[index].set(counts[index].get() + 1));
}
'''
        source = replace(source, '    let incoming = &shared.workers[local.identifier].incoming;', '    count(0);\n    let incoming = &shared.workers[local.identifier].incoming;')
        source = replace(source, '    if let Some(runnable) = local.ready.pop() {', '    if let Some(runnable) = local.ready.pop() {\n        count(1);')
        source = replace(source, '        if self.sleeping.swap(false, Ordering::SeqCst) {', '        if self.sleeping.swap(false, Ordering::SeqCst) {\n            count(7);')
        if label == 'skip-self-wake':
            source = replace(source, '            count(7);', '''            if self.thread.get().expect("sleeping worker has registered its thread").id() == thread::current().id() {
                return true;
            }
            count(7);''')
        source = replace(source, ') {\n    if let Some(active) = driver.as_mut() {', ') {\n    count(if timeout == Some(Duration::ZERO) { 2 } else { 3 });\n    if let Some(active) = driver.as_mut() {')
        source = replace(source, '                worker.sleeping.store(true, Ordering::SeqCst);', '                count(4);\n                worker.sleeping.store(true, Ordering::SeqCst);')
        source = replace(source, '            runnable.run();' if label == 'before' else '                task.run();', '            count(5);\n            runnable.run();' if label == 'before' else '                count(5);\n                task.run();')
        if label != 'before':
            source = replace(source, '            std::mem::swap(&mut local.ready, &mut local.current);', '            count(6);\n            std::mem::swap(&mut local.ready, &mut local.current);')
        source = replace(source, '    CURRENT_WORKER.with(|current| current.take());', '    DIAGNOSTICS.with(|counts| eprintln!("WORKER_COUNTS {:?}", counts.each_ref().map(|count| count.get())));\n    CURRENT_WORKER.with(|current| current.take());')
        worker.write_text(source)
        reactor = checkout / 'crates/iouring/src/io_uring_selector.rs'
        source = reactor.read_text()
        source = replace(source, 'pub(crate) struct Reactor {', 'pub(crate) struct Reactor {\n    diagnostics: [usize; 8],')
        source = replace(source, '            started: false,', '            diagnostics: [0; 8],\n            started: false,')
        source = replace(source, '        self.started = true;', '        self.diagnostics[0] += 1;\n        self.started = true;')
        source = replace(source, '            match command {', '            self.diagnostics[1] += 1;\n            match command {')
        source = replace(source, '            let identifier = completion.user_data();', '            self.diagnostics[2] += 1;\n            let identifier = completion.user_data();')
        source = replace(source, '            if identifier == NOTIFICATION_TAG {', '            if identifier == NOTIFICATION_TAG {\n                self.diagnostics[3] += 1;')
        source = replace(source, '            if needs_submit {', '            if needs_submit {\n                self.diagnostics[4] += 1;')
        source = replace(source, '        let result = syscall("wait", || match timeout {', '        self.diagnostics[5] += 1;\n        let result = syscall("wait", || match timeout {')
        source = replace(source, 'impl Drop for Reactor {\n    fn drop(&mut self) {', 'impl Drop for Reactor {\n    fn drop(&mut self) {\n        eprintln!("REACTOR_COUNTS {:?}", self.diagnostics);')
        reactor.write_text(source)
        (output / (label + '.patch')).write_bytes(subprocess.check_output(['git', 'diff'], cwd=checkout))
        with (output / (label + '.build.log')).open('w') as log:
            subprocess.run(['cargo', 'build', '--release', '--locked', '-p', 'core-scheduler-echo-benchmark'], cwd=checkout, stdout=log, stderr=subprocess.STDOUT, check=True)
        binary = output / label
        shutil.copy2(checkout / 'target/release/core-scheduler-echo-benchmark', binary)
        builds[label] = dict(base=commit, sha256=hashlib.sha256(binary.read_bytes()).hexdigest())
        print('Built counters', label, flush=True)
        subprocess.run(['git', 'restore', 'crates/native/src/worker.rs', 'crates/iouring/src/io_uring_selector.rs'], cwd=checkout, check=True)
    (output / 'build.json').write_text(json.dumps(builds, indent=2) + '\n')
