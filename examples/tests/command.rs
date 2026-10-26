// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#[test]
fn command_emits_a_validated_sample() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_base-scheduler-examples"))
        .args(["native", "frames", "257", "19", "2", "1024"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let sample: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(sample["bytes"], 257 * 19);
    assert_eq!(sample["recipe"], "frames");
}
