// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::{
    io::Read,
    process::{Child, Command, ExitStatus, Output, Stdio},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

// Turns a hang into a failure. It bounds no expected speed, so it is far longer than any run needs.
const WATCHDOG: Duration = Duration::from_secs(60);

// Fail the test, naming it and the step it reached.
fn expired(step: &str) -> ! {
    panic!(
        "{}: test watchdog expired after {WATCHDOG:?} while {step}",
        thread::current().name().unwrap_or("unnamed test")
    )
}

// Wait for a process to exit, killing it if the watchdog expires first.
fn wait(child: &mut Child, step: &str) -> ExitStatus {
    let deadline = Instant::now() + WATCHDOG;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            expired(step);
        }
        thread::sleep(Duration::from_millis(5));
    }
}

// Join a thread, failing if the watchdog expires first.
fn join<T>(thread: JoinHandle<T>, step: &str) -> T {
    let deadline = Instant::now() + WATCHDOG;
    while !thread.is_finished() {
        if Instant::now() >= deadline {
            expired(step);
        }
        thread::sleep(Duration::from_millis(5));
    }
    thread.join().unwrap()
}

trait BoundedOutput {
    // `Command::output`, killing the process and failing the test if the watchdog expires first.
    fn bounded_output(&mut self) -> Output;
}

impl BoundedOutput for Command {
    fn bounded_output(&mut self) -> Output {
        let step = format!(
            "waiting for the benchmark process {:?}",
            self.get_args().collect::<Vec<_>>()
        );
        let mut child = self
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let drain = |mut pipe: Box<dyn Read + Send>| {
            thread::spawn(move || {
                let mut bytes = Vec::new();
                pipe.read_to_end(&mut bytes).unwrap();
                bytes
            })
        };
        let stdout = drain(Box::new(child.stdout.take().unwrap()));
        let stderr = drain(Box::new(child.stderr.take().unwrap()));
        let status = wait(&mut child, &step);
        Output {
            status,
            stdout: join(stdout, &step),
            stderr: join(stderr, &step),
        }
    }
}

#[test]
fn benchmark_binary_emits_a_parseable_sample() {
    let output = Command::new(env!("CARGO_BIN_EXE_base-scheduler-benchmark-files"))
        .args(["native", "monitor", "read-at", "1", "10", "1"])
        .bounded_output();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let sample: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(sample["operations"], 10);
    assert_eq!(sample["mode"], "monitor");
}

#[test]
fn migration_binary_runs_the_controlled_handoff_workload() {
    for args in [
        vec![],
        vec!["2", "2", "4", "4096", "read"],
        vec!["2", "2", "4", "4096", "write", "unregistered"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_migration"))
            .args(&args)
            .bounded_output();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        #[cfg(target_os = "linux")]
        {
            let sample: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(
                sample["operations"],
                if args.is_empty() { 64000 } else { 8 }
            );
        }
        #[cfg(not(target_os = "linux"))]
        assert!(String::from_utf8_lossy(&output.stdout).contains("requires Linux"));
    }
}
