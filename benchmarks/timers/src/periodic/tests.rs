// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::test_support::{self, TestSource, step};

fn options(
    backend: NativeBackend,
    alignment: Alignment,
    delay_us: u64,
    measure: Measure,
) -> PeriodicOptions {
    PeriodicOptions {
        backend,
        workers: 2,
        tasks: 4,
        delay: Duration::from_micros(delay_us),
        alignment,
        measure,
        work: 10,
    }
}

fn window(start: Instant, end: End) -> Window {
    Window {
        start,
        end,
        delay: Duration::from_millis(1),
        aligned: false,
        work: 0,
    }
}

// What `window` records for an expiry of `target` observed at `now`: its overshoot, or nothing.
fn recorded(window: &Window, target: Instant, now: Instant) -> Vec<u64> {
    let mut overshoot = Vec::new();
    window.record(&mut overshoot, target, now);
    overshoot
}

#[test]
fn a_timed_window_includes_expiries_due_after_its_start_and_observed_before_its_end() {
    let start = Instant::now();
    let end = start + Duration::from_secs(3600);
    let open = window(start, End::At(end));
    // The number recorded never ends a timed window.
    assert!(!open.ended(usize::MAX));
    assert_eq!(recorded(&open, start, start), [0]);
    assert_eq!(recorded(&open, start, start + Duration::from_nanos(5)), [5]);
    assert!(recorded(&open, start - Duration::from_nanos(1), start).is_empty());
    assert!(recorded(&open, start, end).is_empty());
    // One expiry per delay, and one more for a deadline at the start.
    assert_eq!(open.capacity(), 3_600_001);
    assert!(window(start, End::At(start)).ended(0));
}

#[test]
fn a_window_of_expiries_ends_at_its_count_and_includes_every_later_deadline() {
    let start = Instant::now();
    let counted = window(start, End::After(2));
    let late = start + Duration::from_secs(3600);
    assert!(!counted.ended(1));
    assert!(counted.ended(2));
    assert_eq!(recorded(&counted, start, late), [3_600_000_000_000]);
    assert!(recorded(&counted, start - Duration::from_nanos(1), late).is_empty());
    assert_eq!(counted.capacity(), 2);
}

#[test]
fn backends_and_alignments_count_expiries_inside_the_window() {
    test_support::run(|| {
        for backend in NativeBackend::ALL {
            for alignment in Alignment::ALL {
                step(format!("measuring {backend:?} with {alignment:?} timers"));
                // A window of expiries, so that however slowly the host runs the tasks, each records two.
                let sample = measure(
                    &options(*backend, *alignment, 1000, Measure::Expiries(2)),
                    TestSource::default(),
                )
                .unwrap();
                assert_eq!(sample.operations, 4 * 2);
                assert!(sample.metric("overshoot_p50").is_some());
                assert!(sample.metric("cpu_time").is_some());
                assert_eq!(
                    sample.metric("rescued_timers").is_some(),
                    cfg!(feature = "diagnostics")
                );
            }
        }
    });
}

#[test]
fn a_window_without_an_expiry_is_an_error() {
    test_support::run(|| {
        let error = measure(
            &options(
                NativeBackend::Native,
                Alignment::Unaligned,
                5000,
                Measure::Duration(Duration::from_millis(1)),
            ),
            TestSource::default(),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "no timer expired inside the measurement window"
        );
    });
}

#[test]
fn a_failed_measurement_still_runs_the_tasks_to_the_end_of_the_window() {
    test_support::run(|| {
        let error = measure(
            &options(
                NativeBackend::Native,
                Alignment::Unaligned,
                1000,
                Measure::Duration(Duration::from_millis(20)),
            ),
            TestSource::failing(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("no resource usage"));
        let error = measure(
            &options(
                NativeBackend::Native,
                Alignment::Unaligned,
                1000,
                Measure::Expiries(1),
            ),
            TestSource::failing(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("no resource usage"));
    });
}
