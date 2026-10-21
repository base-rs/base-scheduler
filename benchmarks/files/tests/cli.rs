// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#[test]
fn benchmark_binary_emits_a_parseable_sample() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_core-scheduler-file-benchmark"))
        .args(["native", "monitor", "read-at", "1", "10", "1"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let sample: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(sample["operations"], 10);
    assert_eq!(sample["mode"], "monitor");
}
