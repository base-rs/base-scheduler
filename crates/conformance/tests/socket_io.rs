// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use base_scheduler::{Cancellation, Cancelled, OperationResult};
use base_scheduler_conformance::{SocketIoFixture, SocketIoGuard, socket};
use futures_channel::oneshot;
use futures_lite::future::block_on;
use std::{
    future::{Future, poll_fn},
    panic::{AssertUnwindSafe, catch_unwind},
    pin::{Pin, pin},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    task::Poll,
    time::Duration,
};

const DEADLINE: Duration = Duration::from_secs(10);

// Owned operation input whose destruction the fixture observes.
struct Input {
    bytes: Vec<u8>,
    dropped: Arc<AtomicBool>,
}
impl Drop for Input {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

// Dropping the release sender also lets a held attempt finish after an assertion fails.
struct Guard {
    release: Option<mpsc::Sender<()>>,
    retired: mpsc::Receiver<()>,
}
impl SocketIoGuard for Guard {
    fn release(&mut self) {
        self.release.take().unwrap().send(()).unwrap();
    }
    async fn retired(&mut self) {
        self.retired.recv_timeout(DEADLINE).unwrap();
    }
}

type Backend = (oneshot::Sender<()>, mpsc::Receiver<()>, mpsc::Sender<()>);

#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    // Leave the input with the waiter, so abandonment destroys it while backend access remains.
    RetainInWaiter,
    // Let cancellation replace the result of an attempt that has already started.
    CancelWins,
}

// A controlled backend: each accepted attempt runs on its own thread, which holds the input at a known boundary until released. Readiness backends cannot provide this boundary: they perform each attempt synchronously inside the polling future, so holding it would also block the poll that must observe it.
struct Fixture {
    input: Mutex<Option<Input>>,
    pointer: usize,
    dropped: Arc<AtomicBool>,
    backend: Mutex<Option<Backend>>,
    guard: Mutex<Option<(oneshot::Receiver<()>, Guard)>>,
    fault: Fault,
}
impl Fixture {
    fn new(fault: Fault) -> Self {
        let dropped = Arc::new(AtomicBool::new(false));
        let input = Input {
            bytes: vec![1, 2, 3],
            dropped: Arc::clone(&dropped),
        };
        let (started, start) = oneshot::channel();
        let (release, released) = mpsc::channel();
        let (retired, retirement) = mpsc::channel();
        Self {
            pointer: input.bytes.as_ptr() as usize,
            input: Mutex::new(Some(input)),
            dropped,
            backend: Mutex::new(Some((started, released, retired))),
            guard: Mutex::new(Some((
                start,
                Guard {
                    release: Some(release),
                    retired: retirement,
                },
            ))),
            fault,
        }
    }
}
impl SocketIoFixture for Fixture {
    type Input = Input;
    type Output = usize;
    type Guard = Guard;
    fn input(&self) -> Input {
        self.input.lock().unwrap().take().unwrap()
    }
    fn assert_retained(&self) {
        assert!(!self.dropped.load(Ordering::SeqCst));
    }
    fn assert_released(&self) {
        assert!(self.dropped.load(Ordering::SeqCst));
    }
    fn assert_completed(&self, bytes: &usize, input: &Input) {
        assert_eq!(*bytes, input.bytes.len());
        assert_eq!(input.bytes.as_ptr() as usize, self.pointer);
    }
    async fn operation(
        &self,
        input: Input,
        cancellation: &Cancellation,
    ) -> (OperationResult<usize>, Input) {
        // Require more than the initial poll to submit, exercising lifecycle assertions under delayed admission.
        futures_lite::future::yield_now().await;
        let (started, released, retired) = self.backend.lock().unwrap().take().unwrap();
        let count = input.bytes.len();
        let (kept, accepted) = if self.fault != Fault::None {
            (Some(input), None)
        } else {
            (None, Some(input))
        };
        let (complete, completion) = oneshot::channel();
        std::thread::spawn(move || {
            started.send(()).unwrap();
            let _ = released.recv_timeout(DEADLINE);
            // An abandoned waiter refuses the input, which is destroyed here before retirement is reported.
            let _ = complete.send(accepted);
            let _ = retired.send(());
        });
        let mut completion = completion;
        let mut cancelled = pin!(cancellation.cancelled());
        let mut observed = false;
        // These cases cancel only after the attempt has started, when its actual result must win. Observe cancellation first so that rule is exercised even when the result is already available.
        let accepted = poll_fn(|context| {
            if !observed && cancelled.as_mut().poll(context).is_ready() {
                observed = true;
                if self.fault == Fault::CancelWins {
                    return Poll::Ready(None);
                }
            }
            Pin::new(&mut completion)
                .poll(context)
                .map(|accepted| Some(accepted.unwrap()))
        })
        .await;
        match accepted {
            Some(accepted) => (Ok(count), kept.or(accepted).unwrap()),
            None => (Err(Cancelled.into()), kept.unwrap()),
        }
    }
    async fn start(&self) -> Guard {
        let (start, guard) = self.guard.lock().unwrap().take().unwrap();
        start.await.unwrap();
        guard
    }
}

fn run(test: impl AsyncFnOnce(&Fixture) + Send + 'static) {
    let (done, completion) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(catch_unwind(AssertUnwindSafe(|| {
            block_on(test(&Fixture::new(Fault::None)))
        })));
    });
    if let Err(payload) = completion
        .recv_timeout(DEADLINE)
        .expect("conformance test deadline exceeded")
    {
        std::panic::resume_unwind(payload);
    }
}

mod lifecycle {
    use super::*;
    base_scheduler_conformance::socket_io_tests!(run: run);
}

#[test]
fn rejects_input_destroyed_with_an_abandoned_waiter() {
    let fixture = Fixture::new(Fault::RetainInWaiter);
    let result = catch_unwind(AssertUnwindSafe(|| {
        block_on(socket::abandoned_operation(&fixture))
    }));
    assert!(result.is_err(), "conformance accepted an incorrect backend");
}

#[test]
fn rejects_cancellation_replacing_a_started_result() {
    let fixture = Fixture::new(Fault::CancelWins);
    let result = catch_unwind(AssertUnwindSafe(|| {
        block_on(socket::completion_wins(&fixture))
    }));
    assert!(result.is_err(), "conformance accepted an incorrect backend");
}
