#!/usr/bin/env python3
"""Build benchmark-only controls, without changing production scheduler code."""
import fcntl
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

root = Path.home() / '.cache/core-scheduler-single-worker-research'
checkout = Path.home() / 'Developer/core-rs/core-scheduler-single-worker-research'
assert not subprocess.check_output(['git', 'status', '--porcelain'], cwd=checkout, text=True)
server = checkout / 'benchmarks/echo/src/server.rs'
text = server.read_text()
insert = '''
// Diagnostic only: compare conservative readiness semantics with native Tokio reads,
// independently of adapter task ownership. No short-read readiness clearing.
async fn tokio_policy_server<const CONSERVATIVE: bool, const ADAPTER_TASKS: bool>(
    listener: TcpListener,
    connections: usize,
) {
    listener.set_nonblocking(true).unwrap();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    let scheduler = TokioScheduler::new(tokio::runtime::Handle::current());
    let mut raw_tasks = Vec::new();
    let mut adapter_tasks = Vec::new();
    for _ in 0..connections {
        let (mut socket, _) = listener.accept().await.unwrap();
        socket.set_nodelay(true).unwrap();
        let future = async move {
            let mut buffer = vec![0; BUFFER_SIZE];
            loop {
                let count = if CONSERVATIVE {
                    loop {
                        socket.readable().await.unwrap();
                        match socket.try_read(&mut buffer) {
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                            result => break result.unwrap(),
                        }
                    }
                } else {
                    socket.read(&mut buffer).await.unwrap()
                };
                if count == 0 { return; }
                socket.write_all(&buffer[..count]).await.unwrap();
            }
        };
        if ADAPTER_TASKS {
            adapter_tasks.push(scheduler.spawn(future).unwrap());
        } else {
            raw_tasks.push(tokio::spawn(future));
        }
    }
    for task in raw_tasks { task.await.unwrap(); }
    for task in adapter_tasks { task.await.unwrap(); }
    scheduler.shutdown().await;
}

async fn adapter_raw_task_server(
    scheduler: Arc<TokioScheduler>, listener: TcpListener, connections: usize, poll: bool,
) {
    let listener = scheduler.register_listener(listener).unwrap();
    let never = Cancellation::never();
    let mut tasks = Vec::new();
    for _ in 0..connections {
        let (socket, _) = scheduler.socket_accept(&listener, &never).await.unwrap();
        scheduler.socket_set_option(&socket, TcpNoDelay, true, &never).await.unwrap();
        let shared = Arc::clone(&scheduler);
        let task = if poll {
            tokio::spawn(async move { echo_poll(shared.as_ref(), socket).await })
        } else {
            tokio::spawn(async move { echo(shared.as_ref(), socket).await })
        };
        tasks.push(task);
    }
    for task in tasks { task.await.unwrap(); }
}
'''
text = text.replace('pub(crate) fn run(', insert + '\npub(crate) fn run(', 1)
arms = '''
        "tokio-policy-native" | "tokio-policy-conservative" | "tokio-policy-native-adapter-task" | "tokio-policy-conservative-adapter-task" => {
            let runtime = crate::runtime(workers);
            ready(&listener);
            match backend {
                "tokio-policy-native" => runtime.block_on(tokio_policy_server::<false, false>(listener, connections)),
                "tokio-policy-conservative" => runtime.block_on(tokio_policy_server::<true, false>(listener, connections)),
                "tokio-policy-native-adapter-task" => runtime.block_on(tokio_policy_server::<false, true>(listener, connections)),
                _ => runtime.block_on(tokio_policy_server::<true, true>(listener, connections)),
            }
        }
        "tokio-adapter-raw-task" | "tokio-adapter-poll-raw-task" => {
            let runtime = crate::runtime(workers);
            let scheduler = Arc::new(TokioScheduler::new(runtime.handle().clone()));
            ready(&listener);
            runtime.block_on(adapter_raw_task_server(Arc::clone(&scheduler), listener, connections, backend == "tokio-adapter-poll-raw-task"));
            runtime.block_on(scheduler.shutdown());
        }
        "native-readiness" | "native-readiness-poll" => {
            use core_scheduler_native::NativeScheduler;
            let scheduler = Arc::new(NativeScheduler::with_options(core_scheduler_native::NativeOptions { workers, ..Default::default() }).unwrap());
            ready(&listener);
            scheduler.block_on(scheduler_server(Arc::clone(&scheduler), listener, connections, backend == "native-readiness-poll"));
            scheduler.block_on(scheduler.shutdown());
        }
'''
text = text.replace('    match backend {\n        "tokio" => {', '    match backend {\n' + arms + '\n        "tokio" => {', 1)
assert text != server.read_text()
server.write_text(text)
with open('/tmp/core-scheduler-perf.lock', 'a') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    with (root / 'controls-build.log').open('w') as output:
        subprocess.run(['cargo', 'build', '--release', '--locked', '-p', 'core-scheduler-echo-benchmark'], cwd=checkout, stdout=output, stderr=subprocess.STDOUT, check=True)
    binary = root / 'controls'
    shutil.copy2(checkout / 'target/release/core-scheduler-echo-benchmark', binary)
patch = subprocess.check_output(['git', 'diff'], cwd=checkout, text=True)
(root / 'controls.patch').write_text(patch)
(root / 'controls-build.json').write_text(json.dumps({
    'base_source': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=checkout, text=True).strip(),
    'patch_sha256': hashlib.sha256(patch.encode()).hexdigest(),
    'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
    'build_command': 'cargo build --release --locked -p core-scheduler-echo-benchmark',
    'production_changes': 'None. Benchmark-only semantic and task-wrapper controls.',
}, indent=2) + '\n')
print('Built', binary, flush=True)
