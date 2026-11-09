// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::{
    io::Read,
    process::{Child, Command, ExitStatus, Output, Stdio},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const BINARY: &str = env!("CARGO_BIN_EXE_base-scheduler-benchmark-timers");
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

#[cfg(unix)]
#[test]
fn the_binary_writes_one_record() {
    let output = Command::new(BINARY)
        .args([
            "--workload",
            "refresh",
            "--mode",
            "after",
            "--iterations",
            "10",
        ])
        .bounded_output();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).lines().count(), 1);
    let record: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(record["samples"][0]["operations"], 10);
    assert_eq!(record["parameters"]["mode"], "after");
}

// Process resource usage comes from `getrusage`, so elsewhere every measurement fails.
#[cfg(not(unix))]
#[test]
fn the_binary_fails_without_process_resource_usage() {
    let output = Command::new(BINARY)
        .args(["--iterations", "10"])
        .bounded_output();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("getrusage"));
}

#[test]
fn invalid_options_fail_without_a_record() {
    let output = Command::new(BINARY).arg("--bad").bounded_output();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing value for --bad"));
    let output = Command::new(BINARY).args(["--bad", "1"]).bounded_output();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown option --bad"));
}

/// The shared runner accepts this workload's records for every declared scenario and variant that this build's features can run.
#[cfg(target_os = "linux")]
#[test]
fn the_shared_runner_smokes_every_scenario() {
    use base_benchmark::Workspace;
    let workspace = Workspace::new(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));
    let package = Some("base-scheduler-benchmark-timers");
    // The features this binary was built with; scenarios that need others are skipped.
    let features: Vec<&str> = [
        cfg!(feature = "allocations").then_some("allocations"),
        cfg!(feature = "diagnostics").then_some("diagnostics"),
    ]
    .into_iter()
    .flatten()
    .collect();
    let scenarios: Vec<_> = workspace
        .suite(package)
        .unwrap()
        .scenarios
        .into_iter()
        .filter(|scenario| {
            scenario
                .features
                .iter()
                .all(|feature| features.contains(&feature.as_str()))
        })
        .collect();
    // 19 timed scenarios, `alloc` with `allocations`, and three rescue-count scenarios with `diagnostics`.
    let expected = 19
        + usize::from(cfg!(feature = "allocations"))
        + 3 * usize::from(cfg!(feature = "diagnostics"));
    assert_eq!(scenarios.len(), expected);
    let directory = tempfile::tempdir().unwrap();
    let campaigns = workspace
        .smoke_executable(
            package,
            std::path::Path::new(BINARY),
            &features,
            &directory.path().join("smoke"),
            Some(directory.path().join("lock")),
        )
        .unwrap();
    assert_eq!(campaigns.len(), scenarios.len());
    for (campaign, scenario) in campaigns.iter().zip(&scenarios) {
        assert_eq!(campaign.summary.scenario, scenario.name);
        assert_eq!(
            campaign.summary.subjects.len(),
            scenario.variants.len().max(1),
            "{}",
            scenario.name
        );
    }
}
