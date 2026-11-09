// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::test_support::{self, TestSource, step};
use std::sync::atomic::{AtomicUsize, Ordering};

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
    test_support::run(|| {
        for workload in ScenarioWorkload::ALL {
            step(format!("measuring {workload:?}"));
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
    });
}

/// A stall that holds its worker until the root thread has measured its sleeps, rather than for a time.
#[derive(Default)]
struct Gated {
    holding: AtomicUsize,
    released: AtomicUsize,
}

impl Stall for Gated {
    fn hold(&self) {
        let stall = self.holding.fetch_add(1, Ordering::AcqRel) + 1;
        while self.released.load(Ordering::Acquire) < stall {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn held(&self) {
        step("waiting for the stalling task to hold its worker");
        while self.holding.load(Ordering::Acquire) == self.released.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(1));
        }
        step("waiting for the root sleeps while the stalling task holds its worker");
    }
    fn measured(&self) {
        self.released.fetch_add(1, Ordering::AcqRel);
        step("joining the stalling task");
    }
}

// Whether stall rescue fired a timer. Without the `diagnostics` feature there are no counts, so each attempt is taken to have exercised rescue.
fn rescued(sample: &Sample) -> bool {
    #[cfg(feature = "diagnostics")]
    {
        matches!(
            sample.metric("rescued_timers").unwrap().measurement,
            base_benchmark::Measurement::Total { value } if value > 0
        )
    }
    #[cfg(not(feature = "diagnostics"))]
    {
        let _ = sample;
        true
    }
}

#[test]
fn a_stalled_worker_does_not_hold_root_sleeps() {
    // Each stalling task holds its worker until the root thread has measured all eight of its sleeps. Root registrations go to each worker in turn, so some sleeps land on the held worker: they can only finish through stall rescue, and a sleep the stall held would never finish, failing the test at the watchdog. A sleep first polled after its deadline finishes without registering and shows nothing, so with rescue counts, repeat until one was rescued; the watchdog bounds the repetition.
    test_support::run(|| {
        for attempt in 1.. {
            let sample = measure_with(
                &options(ScenarioWorkload::Stall, 2, 1000),
                TestSource::default(),
                Arc::new(Gated::default()),
            )
            .unwrap();
            assert_eq!(sample.operations, 2);
            if rescued(&sample) {
                break;
            }
            step(format!("starting attempt {}", attempt + 1));
        }
    });
}

#[test]
fn a_failed_measurement_still_shuts_the_scheduler_down() {
    test_support::run(|| {
        for source in [TestSource::failing(), TestSource::failing_after(1)] {
            let error = measure(&options(ScenarioWorkload::Spawn, 1, 10), source).unwrap_err();
            assert!(error.to_string().contains("no resource usage"));
        }
    });
}
