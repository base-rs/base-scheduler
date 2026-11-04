// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

fn scenario(values: &[&str]) -> serde_json::Value {
    execute(
        &values
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>(),
    )
}

#[test]
fn every_workload_runs_with_a_short_unit() {
    for (workload, samples) in [
        ("root-sleep", 2),
        ("spawn", 0),
        ("spawn-pending", 0),
        ("task-sleep", 2 * 2 * 4),
        ("task-sleep-pending", 2 * 2 * 4),
        ("idle", 0),
        ("pulse", 0),
        ("stall", 2 * 8),
        ("yield", 0),
    ] {
        let record = scenario(&[workload, "2", "2", "100"]);
        assert_eq!(record["workload"], workload);
        assert_eq!(record["iterations"], 2);
        assert_eq!(record["unit_us"], 100);
        assert_eq!(record["samples"], samples, "{workload}");
        assert!(record["ns_per_op"].as_f64().unwrap() > 0.0);
        assert_eq!(record["p50"].is_null(), samples == 0, "{workload}");
        assert_eq!(record["rescues"].is_object(), cfg!(feature = "diagnostics"));
    }
}

#[test]
fn a_stalled_worker_does_not_hold_root_sleeps() {
    // At the default unit, one worker blocks for 200 ms while root sleeps of 10 ms are measured; stall rescue keeps each far below the block.
    let record = scenario(&["stall", "2", "1"]);
    assert_eq!(record["unit_us"], 1000);
    let max = record["max"].as_u64().unwrap();
    assert!(max < 100_000_000, "a root sleep was {max} ns late");
    #[cfg(feature = "diagnostics")]
    assert!(
        record["rescues"]["timers"].as_u64().unwrap() > 0,
        "{record}"
    );
}

#[test]
#[should_panic(expected = "unknown workload")]
fn an_unknown_workload_is_rejected() {
    scenario(&["unknown", "1", "1", "100"]);
}
