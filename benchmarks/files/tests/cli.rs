// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#[test]
fn benchmark_binary_emits_a_parseable_sample() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_base-scheduler-file-benchmark"))
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

#[test]
fn migration_binary_runs_the_controlled_handoff_workload() {
    for args in [
        vec![],
        vec!["2", "2", "4", "4096", "read"],
        vec!["2", "2", "4", "4096", "write", "unregistered"],
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_migration"))
            .args(&args)
            .output()
            .unwrap();
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
