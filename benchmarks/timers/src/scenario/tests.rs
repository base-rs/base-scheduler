// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::test_support::TestSource;
use base_benchmark::Measurement;

fn options(workload: ScenarioWorkload, iterations: usize, unit_us: u64) -> ScenarioOptions {
    ScenarioOptions {
        workload,
        workers: 2,
        iterations,
        unit: Duration::from_micros(unit_us),
    }
}

#[test]
fn every_workload_runs_with_a_short_unit() {
    for workload in ScenarioWorkload::ALL {
        let sample = measure(&options(*workload, 2, 100), TestSource::default()).unwrap();
        assert_eq!(sample.operations, 2);
        assert!(sample.metric("wall_time").is_some());
        let sleeps = matches!(
            workload,
            ScenarioWorkload::RootSleep
                | ScenarioWorkload::TaskSleep
                | ScenarioWorkload::TaskSleepPending
                | ScenarioWorkload::Stall
        );
        assert_eq!(
            sample.metric("overshoot_p50").is_some(),
            sleeps,
            "{workload:?}"
        );
        assert_eq!(
            sample.metric("rescued_timers").is_some(),
            cfg!(feature = "diagnostics")
        );
    }
}

#[test]
fn a_stalled_worker_does_not_hold_root_sleeps() {
    // At the default unit, one worker blocks for 200 ms while root sleeps of 10 ms are measured; stall rescue keeps each far below the block.
    let sample = measure(
        &options(ScenarioWorkload::Stall, 1, 1000),
        TestSource::default(),
    )
    .unwrap();
    assert!(
        matches!(
            sample.metric("overshoot_max").unwrap().measurement,
            Measurement::Mean { value } if value < 100_000_000.0
        ),
        "a root sleep was too late: {sample:?}"
    );
    #[cfg(feature = "diagnostics")]
    assert!(
        matches!(
            sample.metric("rescued_timers").unwrap().measurement,
            Measurement::Total { value } if value > 0
        ),
        "{sample:?}"
    );
}

#[test]
fn a_failed_measurement_still_shuts_the_scheduler_down() {
    for source in [TestSource::failing(), TestSource::failing_after(1)] {
        let error = measure(&options(ScenarioWorkload::Spawn, 1, 10), source).unwrap_err();
        assert!(error.to_string().contains("no resource usage"));
    }
}
