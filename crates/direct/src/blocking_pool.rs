// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{BlockingTask, DirectOptions};
use event_listener::Event;
use futures_channel::oneshot;
use protocol_scheduler::{BlockingError, Cancellation, OperationError, OperationResult, TaskError};
use std::collections::VecDeque;
use std::future::{Future, poll_fn};
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::task::Poll;
use std::thread;

struct Job {
    run: Box<dyn FnOnce() + Send + 'static>,
    submitted: bool,
}

// Field order also releases captures before notifying completion when shutdown discards a queued job.
struct Operation<F, Output> {
    operation: F,
    completion: oneshot::Sender<Result<Output, TaskError>>,
}

impl<F: FnOnce() -> Output, Output> Operation<F, Output> {
    fn run(self, cancelled: bool) {
        let result = if cancelled {
            drop(self.operation);
            Err(TaskError::Cancelled)
        } else {
            catch_unwind(AssertUnwindSafe(self.operation)).map_err(TaskError::Panicked)
        };
        let _ = self.completion.send(result);
    }
}

struct State {
    closed: bool,
    outstanding: usize,
    queue: VecDeque<Job>,
}

struct Shared {
    state: Mutex<State>,
    work: Condvar,
    capacity: Event,
}

/// A scheduler-owned service, independent of its future workers and I/O selector.
pub(crate) struct BlockingPool {
    shared: Arc<Shared>,
    pub(crate) options: DirectOptions,
    started: OnceLock<io::Result<()>>,
    #[cfg(test)]
    pub(crate) fail_start_at: Option<usize>,
}

impl BlockingPool {
    pub(crate) fn new(options: DirectOptions) -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    closed: false,
                    outstanding: 0,
                    queue: VecDeque::new(),
                }),
                work: Condvar::new(),
                capacity: Event::new(),
            }),
            options,
            started: OnceLock::new(),
            #[cfg(test)]
            fail_start_at: None,
        }
    }

    fn start(&self) -> Result<(), BlockingError> {
        self.start_with(|shared, identifier| {
            #[cfg(test)]
            if self.fail_start_at == Some(identifier) {
                return Err(io::Error::other("injected thread startup failure"));
            }
            thread::Builder::new()
                .name(format!("protocol-direct-{identifier}"))
                .spawn(move || work(shared))
                .map(drop)
        })
    }

    fn start_with(
        &self,
        mut spawn: impl FnMut(Arc<Shared>, usize) -> io::Result<()>,
    ) -> Result<(), BlockingError> {
        self.started
            .get_or_init(|| {
                for identifier in 0..self.options.threads {
                    if let Err(error) = spawn(Arc::clone(&self.shared), identifier) {
                        // A partial startup must release any threads already waiting for work.
                        self.close();
                        return Err(error);
                    }
                }
                Ok(())
            })
            .as_ref()
            .copied()
            .map_err(|error| BlockingError::Io(io::Error::new(error.kind(), error.to_string())))
    }

    pub(crate) async fn spawn<F, Output>(
        &self,
        operation: F,
        cancellation: &Cancellation,
    ) -> Result<BlockingTask<Output>, BlockingError>
    where
        F: FnOnce() -> Output + Send + 'static,
        Output: Send + 'static,
    {
        self.admit(cancellation, || {
            let (sender, result) = oneshot::channel();
            let abandoned = Arc::new(AtomicBool::new(false));
            let owner = Arc::clone(&abandoned);
            let cancellation = cancellation.clone();
            let operation = Operation {
                operation,
                completion: sender,
            };
            (
                Job {
                    run: Box::new(move || {
                        operation.run(owner.load(Ordering::Acquire) || cancellation.is_cancelled())
                    }),
                    submitted: false,
                },
                BlockingTask { result, abandoned },
            )
        })
        .await
    }

    /// Admit work whose execution survives waiter abandonment and pool closure. Only scheduler-owned cleanup uses this path.
    pub(crate) async fn submit<F, Output>(
        &self,
        operation: F,
    ) -> Result<oneshot::Receiver<Result<Output, TaskError>>, BlockingError>
    where
        F: FnOnce() -> Output + Send + 'static,
        Output: Send + 'static,
    {
        self.admit(&Cancellation::never(), || {
            let (completion, result) = oneshot::channel();
            let operation = Operation {
                operation,
                completion,
            };
            (
                Job {
                    run: Box::new(move || operation.run(false)),
                    submitted: true,
                },
                result,
            )
        })
        .await
    }

    /// Accepted I/O always retires, returning the original input even on queued cancellation.
    pub(crate) async fn execute<Input, Output>(
        &self,
        input: Input,
        cancellation: &Cancellation,
        operation: impl FnOnce(&mut Input) -> io::Result<Output> + Send + 'static,
    ) -> (OperationResult<Output>, Input)
    where
        Input: Send + 'static,
        Output: Send + 'static,
    {
        let mut input = Some(input);
        let admitted = self
            .admit(cancellation, || {
                let mut input = input.take().expect("admission constructs exactly one job");
                let cancellation = cancellation.clone();
                let (completion, result) = oneshot::channel();
                let job = Job {
                    submitted: true,
                    run: Box::new(move || {
                        let result = catch_unwind(AssertUnwindSafe(|| {
                            let result = cancellation
                                .check()
                                .map_err(OperationError::from)
                                .and_then(|()| operation(&mut input).map_err(OperationError::from));
                            (result, input)
                        }));
                        let _ = completion.send(result);
                    }),
                };
                (job, result)
            })
            .await;
        match admitted {
            Ok(result) => finish_io(result).await,
            Err(error) => (
                Err(admission_error(error)),
                input.expect("failed admission retains its input"),
            ),
        }
    }

    async fn admit<Output>(
        &self,
        cancellation: &Cancellation,
        make_job: impl FnOnce() -> (Job, Output),
    ) -> Result<Output, BlockingError> {
        let mut cancelled = pin!(cancellation.cancelled());
        loop {
            let listener = self.shared.capacity.listen();
            if cancellation.is_cancelled() {
                return Err(BlockingError::Cancelled);
            }
            {
                let mut state = self.shared.state.lock().unwrap();
                if state.closed {
                    return Err(BlockingError::SchedulerClosed);
                }
                if state.outstanding < self.options.capacity {
                    // Initialize without the queue lock: startup failure closes the pool.
                    if self.started.get().is_none() {
                        drop(state);
                        self.start()?;
                        continue;
                    }
                    let (job, result) = make_job();
                    state.outstanding += 1;
                    state.queue.push_back(job);
                    drop(state);
                    self.shared.work.notify_one();
                    return Ok(result);
                }
            }
            let mut listener = pin!(listener);
            poll_fn(|context| {
                if cancelled.as_mut().poll(context).is_ready() {
                    return Poll::Ready(());
                }
                listener.as_mut().poll(context)
            })
            .await;
        }
    }

    pub(crate) async fn drain(&self) {
        loop {
            let listener = self.shared.capacity.listen();
            if self.shared.state.lock().unwrap().outstanding == 0 {
                return;
            }
            listener.await;
        }
    }

    pub(crate) fn close(&self) {
        let queue = {
            let mut state = self.shared.state.lock().unwrap();
            state.closed = true;
            let (submitted, abandoned): (VecDeque<_>, VecDeque<_>) =
                std::mem::take(&mut state.queue)
                    .into_iter()
                    .partition(|job| job.submitted);
            state.queue = submitted;
            abandoned
        };
        self.shared.capacity.notify(usize::MAX);
        self.shared.work.notify_all();
        // Destroy closures and wake their result waiters outside the queue lock. Destructors may reenter the scheduler or panic.
        for job in queue {
            let _ = catch_unwind(AssertUnwindSafe(|| drop(job)));
            self.shared.state.lock().unwrap().outstanding -= 1;
            self.shared.capacity.notify(usize::MAX);
        }
    }
}

impl Drop for BlockingPool {
    fn drop(&mut self) {
        self.close();
    }
}

fn work(shared: Arc<Shared>) {
    loop {
        let job = {
            let mut state = shared.state.lock().unwrap();
            loop {
                if let Some(job) = state.queue.pop_front() {
                    break job;
                }
                if state.closed {
                    return;
                }
                state = shared.work.wait(state).unwrap();
            }
        };
        // Catch abandoned-result and capture destructor panics as well as user execution panics, so capacity is always returned and the worker survives.
        let _ = catch_unwind(AssertUnwindSafe(job.run));
        {
            let mut state = shared.state.lock().unwrap();
            state.outstanding -= 1;
        }
        shared.capacity.notify(usize::MAX);
    }
}

#[cfg(test)]
mod tests;

pub(crate) fn admission_error(error: BlockingError) -> OperationError {
    match error {
        BlockingError::Cancelled => protocol_scheduler::Cancelled.into(),
        BlockingError::SchedulerClosed => {
            io::Error::new(io::ErrorKind::BrokenPipe, "scheduler is closed").into()
        }
        BlockingError::Io(error) => error.into(),
    }
}

async fn finish_io<T>(result: oneshot::Receiver<std::thread::Result<T>>) -> T {
    match result.await.expect("accepted I/O must report completion") {
        Ok(result) => result,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}
