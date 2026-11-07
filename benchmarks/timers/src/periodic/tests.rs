// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::test_support::TestSource;

fn options(
    backend: NativeBackend,
    alignment: Alignment,
    delay_us: u64,
    measure_ms: u64,
) -> PeriodicOptions {
    PeriodicOptions {
        backend,
        workers: 2,
        tasks: 4,
        delay: Duration::from_micros(delay_us),
        alignment,
        measure: Duration::from_millis(measure_ms),
        work: 10,
    }
}

#[test]
fn backends_and_alignments_count_expiries_inside_the_window() {
    for backend in NativeBackend::ALL {
        for alignment in Alignment::ALL {
            // A window several timer ticks long, so that even coarse timers (about 15.6 ms on Windows) record expiries inside it.
            let sample = measure(
                &options(*backend, *alignment, 1000, 150),
                TestSource::default(),
            )
            .unwrap();
            assert!(sample.operations > 0);
            assert!(sample.metric("overshoot_p50").is_some());
            assert!(sample.metric("cpu_time").is_some());
            assert_eq!(
                sample.metric("rescued_timers").is_some(),
                cfg!(feature = "diagnostics")
            );
        }
    }
}

#[test]
fn a_window_without_an_expiry_is_an_error() {
    let error = measure(
        &options(NativeBackend::Native, Alignment::Unaligned, 5000, 1),
        TestSource::default(),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "no timer expired inside the measurement window"
    );
}

#[test]
fn a_failed_measurement_still_runs_the_tasks_to_the_end_of_the_window() {
    let error = measure(
        &options(NativeBackend::Native, Alignment::Unaligned, 1000, 20),
        TestSource::failing(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("no resource usage"));
}
