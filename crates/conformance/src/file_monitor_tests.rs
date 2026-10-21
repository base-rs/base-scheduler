// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// Generate reusable file-monitor cases using the existing file fixture and runner.
#[macro_export]
macro_rules! file_monitor_tests {
    (run: $run:path $(,)?) => {
        $crate::__file_test_cases! { $run;
            sequential_monitor => |fixture, scheduler, file| {
                $crate::file_monitoring::sequential(scheduler, &file).await;
            },
            independent_monitors => |fixture, scheduler, file| {
                $crate::file_monitoring::independent(scheduler, &file).await;
            },
            monitor_cancellation => |fixture, scheduler, file| {
                $crate::file_monitoring::cancellation(scheduler, &file).await;
            },
            abandoned_monitor_operation => |fixture, scheduler, file| {
                $crate::file_monitoring::abandonment(scheduler, &file).await;
            },
        }
    };
}
