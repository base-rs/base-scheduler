// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

mod support;

use base_scheduler::{
    Cancellation, OperationResult, ProcessWait, ProcessWaitFlags, ProcessWaitResult,
};
use std::{
    future::poll_fn,
    sync::atomic::{AtomicBool, Ordering},
    task::Poll,
};

// Model a native notification wait interrupted by shutdown before reaping. The assertion must collect the child's status after this waiter retires.
struct NotificationWaiter {
    closed: AtomicBool,
}

impl ProcessWait for NotificationWaiter {
    #[cfg(unix)]
    type ProcessId = i32;
    #[cfg(windows)]
    type ProcessId = u32;

    async fn process_wait(
        &self,
        _: Self::ProcessId,
        _: ProcessWaitFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<Option<ProcessWaitResult>> {
        cancellation.check()?;
        poll_fn(|_| {
            if self.closed.load(Ordering::Acquire) {
                Poll::Ready(Err(
                    std::io::Error::from(std::io::ErrorKind::BrokenPipe).into()
                ))
            } else {
                Poll::Pending
            }
        })
        .await
    }
}

#[test]
fn shutdown_retirement_without_reaping_preserves_child_status() {
    support::run(|| {
        let waiter = NotificationWaiter {
            closed: AtomicBool::new(false),
        };
        futures_lite::future::block_on(base_scheduler_conformance::process_wait::shutdown(
            &waiter,
            || waiter.closed.store(true, Ordering::Release),
            std::future::ready(()),
        ));
    });
}

// Model a blocking fallback, as a pool-based backend uses: each accepted wait reaps on its own thread. Closure rejects new waits but lets accepted waits collect their status, and an abandoned wait can still reap.
#[cfg(unix)]
struct BlockingWaiter {
    closed: AtomicBool,
    threads: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>,
}

#[cfg(unix)]
impl BlockingWaiter {
    fn new() -> Self {
        Self {
            closed: AtomicBool::new(false),
            threads: std::sync::Mutex::new(Vec::new()),
        }
    }
    fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
    async fn shutdown(&self) {
        self.close();
        for thread in self.threads.lock().unwrap().drain(..) {
            thread.join().unwrap();
        }
    }
}

#[cfg(unix)]
impl ProcessWait for BlockingWaiter {
    type ProcessId = i32;

    async fn process_wait(
        &self,
        process: i32,
        _: ProcessWaitFlags,
        _: &Cancellation,
    ) -> OperationResult<Option<ProcessWaitResult>> {
        if self.closed.load(Ordering::Acquire) {
            return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe).into());
        }
        let (complete, completion) = futures_channel::oneshot::channel();
        self.threads
            .lock()
            .unwrap()
            .push(std::thread::spawn(move || {
                let mut status = 0;
                // SAFETY: `status` is valid writable storage, and the fixture owns this specific child.
                let pid = unsafe { libc::waitpid(process, &mut status, 0) };
                let _ = complete.send((pid, status));
            }));
        let (pid, status) = completion.await.unwrap();
        Ok(Some(ProcessWaitResult {
            pid: pid as u32,
            status: std::os::unix::process::ExitStatusExt::from_raw(status),
        }))
    }
}

#[cfg(unix)]
#[test]
fn accepted_blocking_waits_collect_status_after_closure() {
    support::run(|| {
        let waiter = BlockingWaiter::new();
        futures_lite::future::block_on(base_scheduler_conformance::process_wait::shutdown(
            &waiter,
            || waiter.close(),
            waiter.shutdown(),
        ));
    });
}

#[cfg(unix)]
#[test]
fn abandoned_blocking_waits_may_reap() {
    support::run(|| {
        let waiter = BlockingWaiter::new();
        futures_lite::future::block_on(base_scheduler_conformance::process_wait::abandonment(
            &waiter,
            waiter.shutdown(),
        ));
    });
}

#[test]
#[ignore = "child fixture, invoked by process-wait conformance assertions"]
fn process_child_fixture() {
    base_scheduler_conformance::process_wait::child();
}
