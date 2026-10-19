// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#![cfg(target_os = "linux")]

use core_scheduler::*;
use core_scheduler_conformance::FileTestFixture;
use core_scheduler_iouring::{IoUringOptions, IoUringSelector};
use core_scheduler_native::{NativeExecutorOptions, NativeScheduler};
use futures_lite::future::block_on;
use std::{
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    pin::pin,
    sync::mpsc,
    task::{Context, Poll},
    time::Duration,
};

struct Fixture<S> {
    scheduler: S,
    path: PathBuf,
    _directory: tempfile::TempDir,
}
impl<S: FileScheduler> FileTestFixture for Fixture<S> {
    type Scheduler = S;
    fn scheduler(&self) -> &S {
        &self.scheduler
    }
    fn path(&self) -> &Path {
        &self.path
    }
    fn assert_absent(&self) {
        assert!(!self.path.exists());
    }
    fn assert_contents(&self, expected: &[u8]) {
        assert_eq!(std::fs::read(&self.path).unwrap(), expected);
    }
}
fn fixture<S>(scheduler: S) -> Fixture<S> {
    let directory = tempfile::tempdir().unwrap();
    Fixture {
        scheduler,
        path: directory.path().join("file"),
        _directory: directory,
    }
}
fn watchdog(test: impl FnOnce() + Send + 'static) {
    let (done, completed) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(catch_unwind(AssertUnwindSafe(test)));
    });
    if let Err(panic) = completed
        .recv_timeout(Duration::from_secs(20))
        .expect("selector test timed out")
    {
        std::panic::resume_unwind(panic);
    }
}
fn run_selector(test: impl AsyncFnOnce(&Fixture<IoUringSelector>) + Send + 'static) {
    watchdog(move || {
        let (selector, driver) = IoUringSelector::new(IoUringOptions::default()).unwrap();
        let thread = std::thread::spawn(move || driver.run());
        let fixture = fixture(selector);
        let result = catch_unwind(AssertUnwindSafe(|| block_on(test(&fixture))));
        block_on(fixture.scheduler.shutdown());
        thread.join().unwrap().unwrap();
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    });
}
fn run_native(test: impl AsyncFnOnce(&Fixture<NativeScheduler<IoUringSelector>>) + Send + 'static) {
    watchdog(move || {
        let (selector, driver) = IoUringSelector::new(IoUringOptions::default()).unwrap();
        let thread = std::thread::spawn(move || driver.run());
        let scheduler = NativeScheduler::with_selector(
            selector,
            NativeExecutorOptions {
                workers: 1,
                ..Default::default()
            },
        )
        .unwrap();
        let fixture = fixture(scheduler);
        let result = catch_unwind(AssertUnwindSafe(|| {
            fixture.scheduler.block_on(test(&fixture))
        }));
        block_on(fixture.scheduler.shutdown());
        thread.join().unwrap().unwrap();
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    });
}
mod independent_selector {
    use super::*;
    core_scheduler_conformance::file_tests!(run: run_selector);
}
mod native_composition {
    use super::*;
    core_scheduler_conformance::file_tests!(run: run_native);
}

#[test]
fn one_thread_can_poll_a_borrowing_future_and_drive_the_ring() {
    watchdog(|| {
        let (selector, mut driver) = IoUringSelector::new(IoUringOptions::default()).unwrap();
        let waker = driver.waker();
        let mut context = Context::from_waker(&waker);
        let directory = tempfile::tempdir().unwrap();
        let value = std::rc::Rc::new(b"same-thread".to_vec());
        let mut operation = pin!(async {
            let never = Cancellation::never();
            let file = selector
                .file_open(
                    &directory.path().join("file"),
                    FileOptions {
                        read: true,
                        write: true,
                        create: true,
                        ..Default::default()
                    },
                    &never,
                )
                .await
                .unwrap();
            let (result, _) = selector
                .file_write_at(&file, value.as_ref().clone(), 0, &never)
                .await;
            assert_eq!(result.unwrap(), value.len());
            let (result, buffer) = selector
                .file_read_at(&file, vec![0; value.len()], 0, &never)
                .await;
            assert_eq!(result.unwrap(), value.len());
            assert_eq!(&buffer, value.as_ref());
            selector.file_close(file).await.unwrap();
            selector.shutdown().await;
        });
        loop {
            if let Poll::Ready(()) = operation.as_mut().poll(&mut context) {
                break;
            }
            driver.turn(None).unwrap();
        }
        assert!(driver.turn(None).unwrap());
    });
}

#[test]
fn composed_tasks_clocks_and_socket_fallback_share_one_explicit_scheduler() {
    run_native(async |fixture| {
        let scheduler = &fixture.scheduler;
        assert_eq!(scheduler.spawn(async { 42 }).unwrap().await.unwrap(), 42);
        scheduler
            .sleep(Duration::from_millis(1), &Cancellation::never())
            .await
            .unwrap();
        let (sender, receiver) =
            core_scheduler_conformance::socket::stream_pair(scheduler, SocketDomain::Ipv4).await;
        core_scheduler_conformance::socket::stream(scheduler, &sender, &receiver).await;
        core_scheduler_conformance::socket_messages::stream(scheduler, &sender, &receiver).await;
        scheduler.socket_close(sender).await.unwrap();
        scheduler.socket_close(receiver).await.unwrap();
    });
}
