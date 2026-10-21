// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::task_service::{TaskRegistration, TaskState};
use core_scheduler::TaskError;
use std::{
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    task::{Context, Poll},
};

/// An owning task handle. Dropping it requests destruction after any active poll returns; it never detaches execution.
#[must_use = "await the task or retain its owning handle"]
pub struct NativeTask<T>(pub(crate) async_task::Task<TaskOutcome<T>, TaskState>);

impl<T> Future for NativeTask<T> {
    type Output = Result<T, TaskError>;
    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0)
            .poll(context)
            .map(|mut outcome| outcome.0.take().unwrap())
    }
}

// async-task aborts if its future or stored output destructor unwinds. Keep both user-owned values behind guards, including when the task is abandoned.
pub(crate) struct TaskFuture<F: Future> {
    future: Option<Pin<Box<F>>>,
    _registration: TaskRegistration,
}

impl<F: Future> TaskFuture<F> {
    pub(crate) fn new(future: F, registration: TaskRegistration) -> Self {
        Self {
            future: Some(Box::pin(future)),
            _registration: registration,
        }
    }
}

impl<F: Future> Unpin for TaskFuture<F> {}

impl<F: Future> Future for TaskFuture<F> {
    type Output = TaskOutcome<F::Output>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut result = match catch_unwind(AssertUnwindSafe(|| {
            this.future.as_mut().unwrap().as_mut().poll(context)
        })) {
            Ok(Poll::Pending) => return Poll::Pending,
            Ok(Poll::Ready(output)) => Ok(output),
            Err(payload) => Err(TaskError::Panicked(payload)),
        };
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(this.future.take()))) {
            crate::panic::discard(result);
            result = Err(TaskError::Panicked(payload));
        }
        Poll::Ready(TaskOutcome(Some(result)))
    }
}

impl<F: Future> Drop for TaskFuture<F> {
    fn drop(&mut self) {
        crate::panic::discard(self.future.take());
    }
}

pub(crate) struct TaskOutcome<T>(pub(crate) Option<Result<T, TaskError>>);

impl<T> Drop for TaskOutcome<T> {
    fn drop(&mut self) {
        crate::panic::discard(self.0.take());
    }
}
