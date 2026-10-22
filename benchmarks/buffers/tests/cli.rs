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
