// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread::JoinHandle,
    time::{Duration, Instant},
};

const BINARY: &str = env!("CARGO_BIN_EXE_base-scheduler-benchmark-echo");
// Smoke runs measure until each connection has counted this many requests, so that a slow host cannot leave the window empty.
const REQUESTS: u64 = 8;
const CONNECTIONS: u64 = 4;
const WARMUP_SECONDS: f64 = 0.03;
// Turns a hang into a failure. It bounds no expected speed, so it is far longer than any run needs.
const WATCHDOG: Duration = Duration::from_secs(60);

// Fail the test, naming it and the step it reached.
fn expired(step: &str) -> ! {
    panic!(
        "{}: test watchdog expired after {WATCHDOG:?} while {step}",
        std::thread::current().name().unwrap_or("unnamed test")
    )
}

// Join a thread, failing if the watchdog expires first.
fn join<T>(thread: JoinHandle<T>, step: &str) -> T {
    let until = Instant::now() + WATCHDOG;
    while !thread.is_finished() {
        if Instant::now() >= until {
            expired(step);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    thread.join().unwrap()
}

// A benchmark process and its arguments.
struct Process(Child, String);
impl Process {
    fn new(args: &[&str]) -> Self {
        Self(
            Command::new(BINARY)
                .args(args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
            format!("{args:?}"),
        )
    }
    fn finish(&mut self, success: bool) -> String {
        let until = Instant::now() + WATCHDOG;
        let status = loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= until {
                expired(&format!("waiting for the benchmark process {}", self.1));
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let mut stderr = String::new();
        self.0
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        assert_eq!(status.success(), success, "{stderr}");
        let mut stdout = String::new();
        if let Some(mut stream) = self.0.stdout.take() {
            stream.read_to_string(&mut stdout).unwrap();
        }
        stdout
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn smoke(backend: &str) {
    smoke_with(backend, &[]);
}
fn smoke_with(backend: &str, rate: &[&str]) {
    let mut server = Process::new(&["server", backend, "2", &CONNECTIONS.to_string()]);
    let stdout = server.0.stdout.take().unwrap();
    let (send, recv) = mpsc::channel();
    std::thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        send.send(line).unwrap();
        // Keep reading, so the server can report after serving.
        let mut rest = String::new();
        stdout.read_to_string(&mut rest).unwrap();
        send.send(rest).unwrap();
    });
    let address = recv
        .recv_timeout(WATCHDOG)
        .unwrap_or_else(|_| expired(&format!("waiting for the {backend} server's address")));
    let measure = format!("requests={REQUESTS}");
    let mut client = Process::new(
        &[
            &[
                "client",
                address.trim(),
                "2",
                &CONNECTIONS.to_string(),
                "4096",
                &WARMUP_SECONDS.to_string(),
                &measure,
            ],
            rate,
        ]
        .concat(),
    );
    let output = client.finish(true);
    let stats: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(stats["requests"], CONNECTIONS * REQUESTS);
    assert_eq!(stats["bytes"], 4096);
    assert_eq!(stats["connections"], CONNECTIONS);
    assert!(stats["rtt_p99_us"].as_f64().unwrap() >= stats["rtt_p50_us"].as_f64().unwrap());
    let rate: Option<u64> = rate.first().map(|rate| rate.parse().unwrap());
    assert_eq!(stats["offered_requests_per_second"].as_u64(), rate);
    if let Some(rate) = rate {
        // A paced connection sends its requests one interval apart from the start of the warmup, so its last counted request cannot be sent before REQUESTS - 1 intervals have passed; unpaced ones finish within milliseconds. Allow a microsecond for rounding the interval and warmup to nanoseconds.
        let elapsed =
            stats["warmup_seconds"].as_f64().unwrap() + stats["seconds"].as_f64().unwrap();
        let paced = (REQUESTS - 1) as f64 * CONNECTIONS as f64 / rate as f64;
        assert!(
            elapsed >= paced - 1e-6,
            "{elapsed} s for {paced} s of pacing"
        );
        for field in [
            "paced_send_delay_p50_us",
            "paced_send_delay_p99_us",
            "paced_send_delay_max_us",
        ] {
            assert!(stats[field].as_f64().unwrap() >= 0.0, "{field}");
        }
    } else {
        assert!(stats["paced_send_delay_max_us"].is_null());
    }
    server.finish(true);
    let rest = recv
        .recv_timeout(WATCHDOG)
        .unwrap_or_else(|_| expired(&format!("reading the {backend} server's report")));
    // Native servers end by reporting their rescue counts.
    let rescues = rest
        .lines()
        .last()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap()["rescues"].clone());
    let native = backend.starts_with("native") || backend.starts_with("iouring");
    assert_eq!(rescues.is_some(), native, "{rest}");
    if let Some(rescues) = rescues {
        assert_eq!(
            rescues.is_object(),
            cfg!(feature = "diagnostics"),
            "{rescues}"
        );
    }
}
#[test]
fn pure_tokio() {
    smoke("tokio");
}
#[test]
fn tokio_adapter() {
    smoke("tokio-adapter");
}
#[test]
fn tokio_adapter_poll() {
    smoke("tokio-adapter-poll");
}
#[test]
fn native_readiness() {
    smoke("native");
}
#[test]
fn native_readiness_poll_at_an_offered_rate() {
    smoke_with("native-poll", &["400"]);
}
#[test]
fn native_readiness_vectored() {
    smoke("native-vectored");
}
#[test]
fn native_readiness_owned_vectored() {
    smoke("native-owned-vectored");
}
#[test]
fn native_readiness_pooled() {
    smoke("native-pooled");
}
#[test]
fn tokio_adapter_vectored() {
    smoke("tokio-adapter-vectored");
}
#[test]
fn tokio_adapter_owned_vectored() {
    smoke("tokio-adapter-owned-vectored");
}
#[test]
fn tokio_adapter_pooled() {
    smoke("tokio-adapter-pooled");
}
#[test]
fn native_readiness_monitor_alias() {
    smoke("native-monitor");
}
#[test]
fn offered_rate_reports_paced_requests() {
    smoke_with("tokio", &["400"]);
}
#[cfg(target_os = "linux")]
#[test]
fn native_iouring() {
    smoke("iouring");
}
#[cfg(target_os = "linux")]
#[test]
fn native_iouring_poll() {
    smoke("iouring-poll");
}
#[cfg(target_os = "linux")]
#[test]
fn native_iouring_vectored() {
    smoke("iouring-vectored");
}
#[cfg(target_os = "linux")]
#[test]
fn native_iouring_owned_vectored() {
    smoke("iouring-owned-vectored");
}
#[cfg(target_os = "linux")]
#[test]
fn native_iouring_pooled() {
    smoke("iouring-pooled");
}
#[cfg(target_os = "linux")]
#[test]
fn native_iouring_multishot() {
    smoke("iouring-multishot");
}

#[test]
fn invalid_configuration_fails_before_running_a_workload() {
    for args in [
        vec![],
        vec!["server"],
        vec!["server", "missing", "1", "1"],
        vec!["server", "native-missing", "1", "1"],
        vec!["client", "127.0.0.1:0"],
        vec!["server", "tokio", "0", "1"],
        vec!["server", "tokio", "invalid", "1"],
        vec!["client", "127.0.0.1:0", "1", "1", "65537", "0", "1"],
        vec!["client", "127.0.0.1:0", "1", "1", "1", "0", "0"],
        vec!["client", "127.0.0.1:0", "1", "1", "1", "0", "requests=0"],
        vec!["client", "127.0.0.1:0", "1", "1", "1", "0", "requests=many"],
        vec!["client", "127.0.0.1:0", "1", "1", "1", "-1", "1"],
        vec!["client", "127.0.0.1:0", "1", "1", "1", "NaN", "1"],
        vec!["client", "127.0.0.1:0", "1", "1", "1", "0", "1"],
        vec!["client", "127.0.0.1:0", "1", "1", "1", "0", "1", "0"],
        // Pacing would need an interval shorter than a nanosecond.
        vec![
            "client",
            "127.0.0.1:0",
            "1",
            "1",
            "1",
            "0",
            "1",
            "4000000000",
        ],
    ] {
        Process::new(&args).finish(false);
    }
}

#[test]
fn client_rejects_corrupt_echoes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let thread = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut byte = [0];
        socket.read_exact(&mut byte).unwrap();
        socket.write_all(&[byte[0].wrapping_add(1)]).unwrap();
    });
    Process::new(&["client", &address, "1", "1", "1", "0", "requests=1"]).finish(false);
    join(thread, "waiting for the test server to finish");
}

#[test]
fn client_rejects_an_empty_measurement() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let thread = std::thread::spawn(move || {
        let _ = listener.accept().unwrap();
    });
    Process::new(&["client", &address, "1", "1", "1", "0", "0.000000001"]).finish(false);
    join(thread, "waiting for the test server to finish");
}

#[test]
fn every_client_field_is_validated_and_server_connection_count_must_be_positive() {
    for (field, invalid) in [
        (1, "invalid-address"),
        (2, "0"),
        (3, "0"),
        (4, "0"),
        (5, "invalid-seconds"),
        (6, "invalid-seconds"),
    ] {
        let mut args = vec!["client", "127.0.0.1:0", "1", "1", "1", "0", "1"];
        args[field] = invalid;
        Process::new(&args).finish(false);
    }
    Process::new(&["server", "tokio", "1", "0"]).finish(false);
}

#[test]
fn tokio_adapter_monitor() {
    smoke("tokio-adapter-monitor");
}
#[cfg(target_os = "linux")]
#[test]
fn native_iouring_monitor() {
    smoke("iouring-monitor");
}
