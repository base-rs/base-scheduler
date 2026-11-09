// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    process::{Child, Command, ExitStatus, Output, Stdio},
    sync::mpsc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const BINARY: &str = env!("CARGO_BIN_EXE_base-scheduler-benchmark-writes");
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

// Connect to a server, with reads bounded by the watchdog.
fn connect(address: SocketAddr) -> TcpStream {
    let stream = TcpStream::connect(address).unwrap();
    stream.set_read_timeout(Some(WATCHDOG)).unwrap();
    stream
}

// A server process and its backend.
struct Server(Child, String);

impl Server {
    fn start(backend: &str, connections: usize, send_buffer: usize) -> (Self, SocketAddr) {
        Self::workload(backend, connections, send_buffer, 137, 3)
    }

    fn workload(
        backend: &str,
        connections: usize,
        send_buffer: usize,
        bytes: usize,
        chunks: usize,
    ) -> (Self, SocketAddr) {
        let mut child = Command::new(BINARY)
            .args([
                "server",
                backend,
                "1",
                &connections.to_string(),
                &bytes.to_string(),
                &chunks.to_string(),
                "2",
                &send_buffer.to_string(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, receive) = mpsc::channel();
        thread::spawn(move || {
            let mut address = String::new();
            BufReader::new(stdout).read_line(&mut address).unwrap();
            let _ = send.send(address);
        });
        let server = Self(child, backend.to_owned());
        let address = receive
            .recv_timeout(WATCHDOG)
            .unwrap_or_else(|_| expired(&format!("waiting for the {backend} server's address")));
        (server, address.trim().parse().unwrap())
    }

    fn finish(mut self, success: bool) {
        let step = format!("waiting for the {} server to exit", self.1);
        let status = wait(&mut self.0, &step);
        assert_eq!(status.success(), success);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn backends() -> Vec<&'static str> {
    let mut backends = vec![
        "tokio",
        "tokio-monitor",
        "tokio-monitor-alternating",
        "tokio-writer",
    ];
    #[cfg(target_os = "linux")]
    {
        backends.extend(["native-monitor", "native-monitor-alternating"]);
        if base_scheduler_iouring::IoUringSelector::new(Default::default()).is_ok() {
            backends.extend([
                "iouring-monitor",
                "iouring-monitor-alternating",
                "iouring-writer",
            ]);
        }
    }
    backends.shrink_to_fit();
    backends
}

#[test]
fn every_backend_exchanges_validated_responses_and_rejects_invalid_requests() {
    for backend in backends() {
        let (server, address) = Server::start(backend, 2, 65536);
        let client = Command::new(BINARY)
            .args([
                "client",
                &address.to_string(),
                "2",
                "2",
                "137",
                "3",
                "2",
                "0.02",
                // Measure until each connection has counted four requests, so that a slow host cannot leave the window empty.
                "requests=4",
            ])
            .bounded_output();
        let detail = format!("{backend}: {}", String::from_utf8_lossy(&client.stderr));
        assert!(client.status.success(), "{detail}");
        let report: serde_json::Value = serde_json::from_slice(&client.stdout).unwrap();
        assert_eq!(report["requests"], 2 * 4, "{detail}");
        server.finish(true);

        let (server, address) = Server::start(backend, 1, 0);
        let mut client = connect(address);
        client.write_all(&[3]).unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let mut response = Vec::new();
        client
            .read_to_end(&mut response)
            .unwrap_or_else(|error| expired(&format!("reading the {backend} response ({error})")));
        assert!(response.is_empty());
        server.finish(false);
    }
}

#[test]
fn client_rejects_corrupt_payloads() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0];
        stream.read_exact(&mut request).unwrap();
        stream.write_all(&[0; 137]).unwrap();
    });
    let client = Command::new(BINARY)
        .args([
            "client",
            &address.to_string(),
            "1",
            "1",
            "137",
            "1",
            "0",
            "0",
            "requests=1",
        ])
        .bounded_output();
    join(server, "waiting for the test server to finish");
    assert!(!client.status.success());
    assert!(String::from_utf8_lossy(&client.stderr).contains("payload mismatch"));
}

#[test]
fn invalid_command_lines_fail_without_starting_work() {
    for args in [
        vec!["server", "unknown", "1", "1", "1", "1", "0", "0"],
        vec!["client", "127.0.0.1:1", "1", "1", "1", "1", "0", "0", "0"],
        vec!["client", "invalid", "1", "1", "1", "1", "0", "0", "1"],
        vec!["unknown"],
        vec!["server", "tokio", "0", "1", "1", "1", "0", "0"],
        vec!["server", "tokio", "1", "0", "1", "1", "0", "0"],
        vec!["server", "tokio", "1", "1", "1", "0", "0", "0"],
        vec!["server", "tokio", "1", "1", "1", "1", "0", "invalid"],
        vec![
            "client",
            "127.0.0.1:1",
            "1",
            "1",
            "1",
            "1",
            "0",
            "0",
            "invalid",
        ],
        vec!["client", "127.0.0.1:1", "0", "1", "1", "1", "0", "0", "1"],
        vec!["client", "127.0.0.1:1", "1", "0", "1", "1", "0", "0", "1"],
        vec!["client", "127.0.0.1:1", "1", "1", "1", "0", "0", "0", "1"],
        vec![
            "client",
            "127.0.0.1:1",
            "1",
            "1",
            "1",
            "1",
            "0",
            "invalid",
            "1",
        ],
    ] {
        assert!(
            !Command::new(BINARY)
                .args(args)
                .bounded_output()
                .status
                .success()
        );
    }
}

#[test]
fn connection_refusal_is_reported() {
    // Ephemeral binds never assign a reserved port, so a concurrent test's server cannot be listening on it.
    let client = Command::new(BINARY)
        .args(["client", "127.0.0.1:1", "1", "1", "1", "1", "0", "0", "0.1"])
        .bounded_output();
    assert!(!client.status.success());
    assert!(String::from_utf8_lossy(&client.stderr).contains("refused"));
}

#[test]
fn client_reports_a_response_cut_short_by_disconnect() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0];
        stream.read_exact(&mut request).unwrap();
        stream.write_all(&[69]).unwrap();
    });
    let client = Command::new(BINARY)
        .args([
            "client",
            &address.to_string(),
            "1",
            "1",
            "137",
            "1",
            "0",
            "0",
            "requests=1",
        ])
        .bounded_output();
    join(server, "waiting for the test server to finish");
    assert!(!client.status.success());
    assert!(String::from_utf8_lossy(&client.stderr).contains("early eof"));
}

#[cfg(not(target_os = "linux"))]
#[test]
fn native_backends_report_platform_requirement() {
    let (server, address) = Server::start("native-monitor", 1, 0);
    let _client = TcpStream::connect(address).unwrap();
    server.finish(false);
}

#[test]
fn every_backend_reports_a_reset_peer_during_reading_and_writing() {
    for backend in backends() {
        let (server, address) = Server::start(backend, 1, 0);
        let client = TcpStream::connect(address).unwrap();
        socket2::SockRef::from(&client)
            .set_linger(Some(Duration::ZERO))
            .unwrap();
        drop(client);
        server.finish(false);

        for chunks in [1, 8] {
            let (server, address) = Server::workload(backend, 1, 65536, 1024 * 1024, chunks);
            let mut client = connect(address);
            client.write_all(&[1]).unwrap();
            // The small send buffer keeps the response incomplete while this peer stops receiving, exercising both last-write flush and next-write rejection.
            client.read_exact(&mut [0]).unwrap_or_else(|error| {
                expired(&format!("reading the {backend} response ({error})"))
            });
            socket2::SockRef::from(&client)
                .set_linger(Some(Duration::ZERO))
                .unwrap();
            drop(client);
            server.finish(false);
        }
    }
}
