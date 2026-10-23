// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

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
        let output = std::process::Command::new(binary)
            .args(arguments)
            .output()
            .unwrap();
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
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_registered"))
        .args(["fixed", "tcp", "read", "64", "1", "3", "retained"])
        .output()
        .unwrap();
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
    use std::io::BufRead;
    for transport in ["tcp", "udp"] {
        let mut server = std::process::Command::new(env!("CARGO_BIN_EXE_pipeline"))
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
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = std::io::BufReader::new(server.stdout.take().unwrap());
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        let ready: serde_json::Value = serde_json::from_str(&line).unwrap();
        let client = std::process::Command::new(env!("CARGO_BIN_EXE_pipeline"))
            .args([
                "client",
                transport,
                ready["ready"].as_str().unwrap(),
                "4096",
                "1",
                "4",
            ])
            .output()
            .unwrap();
        assert!(
            client.status.success(),
            "{}",
            String::from_utf8_lossy(&client.stderr)
        );
        let verified: serde_json::Value = serde_json::from_slice(&client.stdout).unwrap();
        assert_eq!(verified["client_verified"], true);
        line.clear();
        output.read_line(&mut line).unwrap();
        let sample: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(sample["bytes"], (4096 - 16) * 4);
        assert!(server.wait().unwrap().success());
    }
}
