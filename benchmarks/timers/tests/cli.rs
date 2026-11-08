// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::process::Command;

const BINARY: &str = env!("CARGO_BIN_EXE_base-scheduler-benchmark-timers");

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
        .output()
        .unwrap();
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
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("getrusage"));
}

#[test]
fn invalid_options_fail_without_a_record() {
    let output = Command::new(BINARY).arg("--bad").output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing value for --bad"));
    let output = Command::new(BINARY).args(["--bad", "1"]).output().unwrap();
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
