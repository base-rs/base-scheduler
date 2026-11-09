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
fn benchmark_binaries_emit_parseable_samples() {
    for (binary, arguments, field, value) in [
        (
            env!("CARGO_BIN_EXE_files"),
            ["native", "submit", "4096", "10001", "1", ""],
            "bytes",
            10001,
        ),
        (
            env!("CARGO_BIN_EXE_sockets"),
            ["native", "shared", "64", "2", "1", "udp"],
            "iterations",
            2,
        ),
        (
            env!("CARGO_BIN_EXE_leases"),
            ["2", "100", "local", "frozen", "", ""],
            "threads",
            2,
        ),
    ] {
        let output = Command::new(binary).args(arguments).bounded_output();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let sample: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(sample[field], value);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn registered_buffer_binary_emits_a_parseable_sample() {
    let output = Command::new(env!("CARGO_BIN_EXE_registered"))
        .args(["fixed", "tcp", "read", "64", "1", "3", "retained"])
        .bounded_output();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let sample: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(sample["mode"], "fixed");
    assert_eq!(sample["bytes"], 192);
}

#[cfg(target_os = "linux")]
#[test]
fn remote_pipeline_client_verifies_frozen_payloads() {
    use std::{
        io::{BufRead, BufReader},
        sync::mpsc,
    };
    // Kills the server if the test fails before it exits.
    struct Server(Child);
    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    for transport in ["tcp", "udp"] {
        let server = Command::new(env!("CARGO_BIN_EXE_pipeline"))
            .args([
                "fixed-zc",
                transport,
                "4096",
                "1",
                "4",
                "retained",
                "handoff",
                "127.0.0.1:0",
            ])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut server = Server(server);
        // Read the server's lines on a thread of their own, so that the watchdog bounds each wait for one.
        let output = BufReader::new(server.0.stdout.take().unwrap());
        let (send, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in output.lines() {
                if send.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        let line = lines.recv_timeout(WATCHDOG).unwrap_or_else(|_| {
            expired(&format!("waiting for the {transport} server to be ready"))
        });
        let ready: serde_json::Value = serde_json::from_str(&line).unwrap();
        let client = Command::new(env!("CARGO_BIN_EXE_pipeline"))
            .args([
                "client",
                transport,
                ready["ready"].as_str().unwrap(),
                "4096",
                "1",
                "4",
            ])
            .bounded_output();
        assert!(
            client.status.success(),
            "{}",
            String::from_utf8_lossy(&client.stderr)
        );
        let verified: serde_json::Value = serde_json::from_slice(&client.stdout).unwrap();
        assert_eq!(verified["client_verified"], true);
        let line = lines
            .recv_timeout(WATCHDOG)
            .unwrap_or_else(|_| expired(&format!("waiting for the {transport} server's sample")));
        let sample: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(sample["bytes"], (4096 - 16) * 4);
        let step = format!("waiting for the {transport} server to exit");
        assert!(wait(&mut server.0, &step).success());
    }
}
