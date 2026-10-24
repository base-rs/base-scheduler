// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// Generate reusable file-monitor cases using the existing file fixture and runner.
#[macro_export]
macro_rules! file_monitor_tests {
    (run: $run:path $(,)?) => {
        $crate::__file_test_cases! { $run;
            bound_file_handles_preserve_cursor_offsets_and_ownership => |fixture, scheduler, file| {
                use $crate::__core_scheduler::{FileReader as _, FileReading as _, FileWriter as _, FileWriting as _};
                let never = $crate::__core_scheduler::Cancellation::never();
                let mut first = scheduler.file_reader(&file).unwrap();
                let mut second = scheduler.file_reader(&file).unwrap();
                let mut writer = scheduler.file_writer(&file).unwrap();
                let storage = ::std::vec![0; 2];
                let pointer = storage.as_ptr();
                let (result, storage) = first.read(storage, &never).await;
                assert_eq!(result.unwrap(), 2);
                assert_eq!(storage, b"ab");
                assert_eq!(storage.as_ptr(), pointer);
                let (result, storage) = second.read(storage, &never).await;
                assert_eq!(result.unwrap(), 2);
                assert_eq!(storage, b"cd");
                let (result, storage) = writer.write(storage, &never).await;
                assert_eq!(result.unwrap(), 2);
                assert_eq!(storage.as_ptr(), pointer);
                fixture.assert_contents(b"abcdcd");
                let (result, storage) = first.read_at(storage, 1, &never).await;
                assert_eq!(result.unwrap(), 2);
                assert_eq!(storage, b"bc");
                assert_eq!(writer.write_at(storage, 0, &never).await.0.unwrap(), 2);
                fixture.assert_contents(b"bccdcd");
                scheduler.file_seek(&file, ::std::io::SeekFrom::Start(0), &never).await.unwrap();
                assert_eq!(writer.write_all(::std::vec![42; 6], &never).await.0.unwrap(), 6);
                fixture.assert_contents(&[42; 6]);
                first.close().await.unwrap();
                second.close().await.unwrap();
                writer.close().await.unwrap();
                first.close().await.unwrap();
                writer.close().await.unwrap();
            },
            bound_file_handles_reject_cancelled_and_closed_operations => |fixture, scheduler, file| {
                use $crate::__core_scheduler::{FileReader as _, FileReading as _, FileWriter as _, FileWriting as _};
                let cancelled = $crate::__core_scheduler::Cancellation::new();
                cancelled.cancel();
                let never = $crate::__core_scheduler::Cancellation::never();
                let mut reader = scheduler.file_reader(&file).unwrap();
                let mut writer = scheduler.file_writer(&file).unwrap();
                for result in [
                    reader.read(::std::vec![42], &cancelled).await,
                    reader.read_at(::std::vec![42], 0, &cancelled).await,
                    writer.write(::std::vec![42], &cancelled).await,
                    writer.write_at(::std::vec![42], 0, &cancelled).await,
                    writer.write_all(::std::vec![42], &cancelled).await,
                ] {
                    assert!(matches!(result.0, Err($crate::__core_scheduler::OperationError::Cancelled(_))));
                    assert_eq!(result.1, [42]);
                }
                fixture.assert_contents(b"abcdef");
                reader.close().await.unwrap();
                writer.close().await.unwrap();
                assert_eq!(reader.read(::std::vec![42], &never).await.0.unwrap_err().kind(), ::std::io::ErrorKind::BrokenPipe);
                assert_eq!(writer.write(::std::vec![42], &never).await.0.unwrap_err().kind(), ::std::io::ErrorKind::BrokenPipe);
            },
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
                use $crate::__core_scheduler::{FileOpen as _, backend::FileMonitoring as _};
                let never = $crate::__core_scheduler::Cancellation::never();
                let read_only = scheduler.file_open(fixture.path(), $crate::__core_scheduler::FileOptions::READ, &never).await.unwrap();
                let mut monitor = scheduler.file_monitor(&read_only).unwrap();
                let (result, buffer) = scheduler.file_monitor_write(&mut monitor, ::std::vec![42], &never).await;
                assert!(result.is_err());
                assert_eq!(buffer, [42]);
                scheduler.file_monitor_close(&mut monitor).await.unwrap();
                drop(monitor);
                scheduler.file_close(read_only).await.unwrap();
            },
            abandoned_monitor_operation => |fixture, scheduler, file| {
                $crate::file_monitoring::abandonment(scheduler, &file).await;
            },
        }
    };
}
