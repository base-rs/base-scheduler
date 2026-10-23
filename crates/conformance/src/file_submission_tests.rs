// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// Portable submission, independently owned receive, and pipeline checks.
#[macro_export]
macro_rules! file_submission_tests {
    (run: $run:path $(,)?) => {
        $crate::__file_test_cases! { $run;
            submission_is_lazy_and_pre_cancelled_admission_returns_the_owner => |fixture, scheduler, file| {
                use $crate::__core_scheduler::FileSubmission as _;
                let never = $crate::__core_scheduler::Cancellation::never();
                let cancelled = $crate::__core_scheduler::Cancellation::new();
                cancelled.cancel();
                drop(scheduler.file_read_submit(&file, ::std::vec![0;3], &never));
                drop(scheduler.file_write_submit(&file, ::std::vec![0;3], &never));
                drop(scheduler.file_read_at_submit(&file, ::std::vec![0;3], 0, &never));
                drop(scheduler.file_write_at_submit(&file, ::std::vec![0;3], 0, &never));
                let (error, buffer) = scheduler
                    .file_read_submit(&file, ::std::vec![7;3], &cancelled)
                    .await
                    .err()
                    .unwrap();
                assert!(matches!(
                    error,
                    $crate::__core_scheduler::OperationError::Cancelled(_)
                ));
                assert_eq!(buffer, [7; 3]);
                assert!(
                    scheduler
                        .file_write_submit(&file, ::std::vec![7;3], &cancelled)
                        .await
                        .is_err()
                );
                assert!(
                    scheduler
                        .file_read_at_submit(&file, ::std::vec![7;3], 0, &cancelled)
                        .await
                        .is_err()
                );
                assert!(
                    scheduler
                        .file_write_at_submit(&file, ::std::vec![7;3], 0, &cancelled)
                        .await
                        .is_err()
                );
                fixture.assert_contents(b"abcdef");
            },
            submitted_reads_and_writes_return_the_original_storage => |fixture, scheduler, file| {
                use $crate::__core_scheduler::FileSubmission as _;
                let never = $crate::__core_scheduler::Cancellation::never();
                let buffer = ::std::vec![0;3];
                let pointer = buffer.as_ptr();
                let completion = scheduler
                    .file_read_submit(&file, buffer, &never)
                    .await
                    .unwrap();
                let (result, buffer) = completion.await;
                assert_eq!(result.unwrap(), 3);
                assert_eq!(buffer, b"abc");
                assert_eq!(buffer.as_ptr(), pointer);
                let (result, buffer) = scheduler
                    .file_read_at_submit(&file, buffer, 3, &never)
                    .await
                    .unwrap()
                    .await;
                assert_eq!(result.unwrap(), 3);
                assert_eq!(buffer, b"def");
                let (result, buffer) = scheduler
                    .file_write_at_submit(&file, buffer, 0, &never)
                    .await
                    .unwrap()
                    .await;
                assert_eq!(result.unwrap(), 3);
                scheduler
                    .file_seek(&file, ::std::io::SeekFrom::Start(3), &never)
                    .await
                    .unwrap();
                let (result, _) = scheduler
                    .file_write_submit(&file, buffer, &never)
                    .await
                    .unwrap()
                    .await;
                assert_eq!(result.unwrap(), 3);
                fixture.assert_contents(b"defdef");
                for reading in [false, true] {
                    let result = if reading {
                        match scheduler
                            .file_read_at_submit(&file, ::std::vec![0;1], u64::MAX, &never)
                            .await
                        {
                            Ok(completion) => completion.await.0,
                            Err((error, _)) => Err(error),
                        }
                    } else {
                        match scheduler
                            .file_write_at_submit(&file, ::std::vec![0;1], u64::MAX, &never)
                            .await
                        {
                            Ok(completion) => completion.await.0,
                            Err((error, _)) => Err(error),
                        }
                    };
                    assert_eq!(
                        result.unwrap_err().kind(),
                        ::std::io::ErrorKind::InvalidInput
                    );
                }
            },
            accepted_write_progresses_without_polling_its_completion => |fixture, scheduler, file| {
                use $crate::__core_scheduler::{FileReadAt as _, FileSubmission as _};
                let never = $crate::__core_scheduler::Cancellation::never();
                let completion = scheduler
                    .file_write_at_submit(&file, b"UVWXYZ".to_vec(), 0, &never)
                    .await
                    .unwrap();
                let deadline = ::std::time::Instant::now() + ::std::time::Duration::from_secs(5);
                loop {
                    let (result, buffer) = scheduler
                        .file_read_at(&file, ::std::vec![0;6], 0, &never)
                        .await;
                    assert_eq!(result.unwrap(), 6);
                    if buffer == b"UVWXYZ" {
                        break;
                    }
                    assert!(
                        ::std::time::Instant::now() < deadline,
                        "accepted write never progressed"
                    );
                }
                let (result, _) = completion.await;
                assert_eq!(result.unwrap(), 6);
            },
            receiver_chunks_survive_the_next_read_and_pool_exhaustion_is_cancellable => |fixture, scheduler, file| {
                use $crate::__core_scheduler::FileMonitoring as _;
                let never = $crate::__core_scheduler::Cancellation::never();
                let pool = $crate::__core_scheduler::HeapBufferPool::new(2, 3).unwrap();
                let mut receiver =
                    $crate::__core_scheduler::FileReceiver::new(scheduler.file_monitor(&file).unwrap(), pool.clone());
                let first = receiver.recv(scheduler, &never).await.unwrap().unwrap();
                let second = receiver.recv(scheduler, &never).await.unwrap().unwrap();
                assert_eq!(first.as_ref(), b"abc");
                assert_eq!(second.as_ref(), b"def");
                assert!(pool.try_acquire().is_none());
                let token = $crate::__core_scheduler::Cancellation::new();
                token.cancel();
                assert!(receiver.recv(scheduler, &token).await.is_err());
                drop(first);
                assert!(receiver.recv(scheduler, &never).await.unwrap().is_none());
                assert!(receiver.recv(scheduler, &never).await.unwrap().is_none());
                receiver.close(scheduler, &never).await.unwrap();
                drop(receiver);
                assert_eq!(second.as_ref(), b"def");
                drop(second);
            },
            dropping_an_unpolled_accepted_completion_releases_only_after_retirement => |fixture, scheduler, file| {
                use $crate::__core_scheduler::FileSubmission as _;
                let never = $crate::__core_scheduler::Cancellation::never();
                let pool = $crate::__core_scheduler::HeapBufferPool::new(1, 6).unwrap();
                let buffer = pool.try_acquire().unwrap();
                let completion = scheduler
                    .file_write_at_submit(&file, buffer, 0, &never)
                    .await
                    .unwrap();
                assert!(pool.try_acquire().is_none());
                drop(completion);
                let returned = pool.acquire(&never).await.unwrap();
                drop(returned);
            },
            receiver_lease_can_be_forwarded_while_the_next_read_is_running => |fixture, scheduler, file| {
                use $crate::__core_scheduler::{FileMonitoring as _, FileOpen as _};
                let never = $crate::__core_scheduler::Cancellation::never();
                let output = scheduler
                    .file_open(
                        &fixture.path().with_extension("copy"),
                        $crate::__core_scheduler::FileOptions::CREATE,
                        &never,
                    )
                    .await
                    .unwrap();
                let pool = $crate::__core_scheduler::HeapBufferPool::new(2, 3).unwrap();
                let mut receiver =
                    $crate::__core_scheduler::FileReceiver::new(scheduler.file_monitor(&file).unwrap(), pool);
                let first = receiver.recv(scheduler, &never).await.unwrap().unwrap();
                let writing = $crate::__core_scheduler::file_write_all_submit(scheduler, &output, first, &never)
                    .await
                    .unwrap();
                let second = receiver.recv(scheduler, &never).await.unwrap().unwrap();
                let (result, first) = writing.await;
                assert_eq!(result.unwrap(), 3);
                drop(first);
                let (result, second) = $crate::__core_scheduler::file_write_all(scheduler, &output, second, &never).await;
                assert_eq!(result.unwrap(), 3);
                drop(second);
                receiver.close(scheduler, &never).await.unwrap();
                drop(receiver);
                scheduler.file_close(output).await.unwrap();
                assert_eq!(
                    ::std::fs::read(fixture.path().with_extension("copy")).unwrap(),
                    b"abcdef"
                );
            },
        }
    };
}
