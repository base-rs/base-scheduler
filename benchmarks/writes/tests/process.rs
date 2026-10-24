// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};

const BINARY: &str = env!("CARGO_BIN_EXE_core-scheduler-write-benchmark");

struct Server(Child);

impl Server {
    fn start(backend: &str, connections: usize, send_buffer: usize) -> (Self, SocketAddr) {
        let mut child = Command::new(BINARY)
            .args([
                "server",
                backend,
                "1",
                &connections.to_string(),
                "137",
                "3",
                "2",
                &send_buffer.to_string(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut address = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        (Self(child), address.trim().parse().unwrap())
    }

    fn finish(mut self, success: bool) {
        assert_eq!(self.0.wait().unwrap().success(), success);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn backends() -> Vec<&'static str> {
    let mut backends = vec!["tokio", "tokio-monitor", "tokio-writer"];
    #[cfg(target_os = "linux")]
    {
        backends.push("native-monitor");
        if core_scheduler_iouring::IoUringSelector::new(Default::default()).is_ok() {
            backends.extend(["iouring-monitor", "iouring-writer"]);
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
                "0.1",
            ])
            .output()
            .unwrap();
        assert!(
            client.status.success(),
            "{backend}: {}",
            String::from_utf8_lossy(&client.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&client.stdout).unwrap();
        assert!(report["requests"].as_u64().unwrap() > 0);
        server.finish(true);

        let (server, address) = Server::start(backend, 1, 0);
        let mut client = TcpStream::connect(address).unwrap();
        client.write_all(&[3]).unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).unwrap();
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
            "0.1",
        ])
        .output()
        .unwrap();
    server.join().unwrap();
    assert!(!client.status.success());
    assert!(String::from_utf8_lossy(&client.stderr).contains("payload mismatch"));
}

#[test]
fn client_excludes_a_response_crossing_the_measurement_boundary() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0];
        stream.read_exact(&mut request).unwrap();
        thread::sleep(Duration::from_millis(20));
        // The one-byte response for request 1 and work 0.
        stream.write_all(&[69]).unwrap();
    });
    let client = Command::new(BINARY)
        .args([
            "client",
            &address.to_string(),
            "1",
            "1",
            "1",
            "1",
            "0",
            "0",
            "0.001",
        ])
        .output()
        .unwrap();
    server.join().unwrap();
    assert!(!client.status.success());
    assert!(String::from_utf8_lossy(&client.stderr).contains("no complete responses"));
}

#[test]
fn invalid_command_lines_fail_without_starting_work() {
    for args in [
        vec!["server", "unknown", "1", "1", "1", "1", "0", "0"],
        vec!["client", "127.0.0.1:1", "1", "1", "1", "1", "0", "0", "0"],
        vec!["client", "invalid", "1", "1", "1", "1", "0", "0", "1"],
        vec!["unknown"],
    ] {
        assert!(
            !Command::new(BINARY)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}
