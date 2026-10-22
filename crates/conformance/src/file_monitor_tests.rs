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
            monitor_inflight_cancellation => |fixture, scheduler, file| {
                $crate::file_monitoring::inflight_cancellation(scheduler, &file).await;
            },
            pending_monitor_retirement => |fixture, scheduler, file| {
                $crate::file_monitoring::pending_retirement(scheduler, &file).await;
            },
            monitor_write_error => |fixture, scheduler, file| {
                use $crate::__core_scheduler::{FileOpen as _, FileMonitoring as _};
                let never = $crate::__core_scheduler::Cancellation::never();
                let read_only = scheduler.file_open(fixture.path(), $crate::__core_scheduler::FileOptions::READ, &never).await.unwrap();
                let mut monitor = scheduler.file_monitor(&read_only).unwrap();
                let (result, buffer) = scheduler.file_monitor_write(&mut monitor, ::std::vec![42], &never).await;
                assert!(result.is_err());
                assert_eq!(buffer, [42]);
                scheduler.file_monitor_close(&mut monitor, &never).await.unwrap();
                drop(monitor);
                scheduler.file_close(read_only).await.unwrap();
            },
            abandoned_monitor_operation => |fixture, scheduler, file| {
                $crate::file_monitoring::abandonment(scheduler, &file).await;
            },
        }
    };
}
