// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::Registration;
use crate::worker_timers;
use crate::{NativeScheduler, Selector, task_service::Shared};
use base_scheduler::{Cancellation, Clock, OperationResult, Timers};
use std::{
    future::{Future, poll_fn},
    io,
    pin::pin,
    sync::Arc,
    task::{Context, Poll, Waker},
    time::Instant,
};

/// Experimental timer resource, constructed through `NativeScheduler::timer`. Pending registrations currently stay in their original worker's queue when the waiting task moves; this is Native placement, not an interface guarantee.
pub struct NativeTimer {
    shared: Arc<Shared>,
    armed: Option<Instant>,
    registration: Option<Registration>,
    waker: Option<Waker>,
}
impl NativeTimer {
    fn disarm(&mut self) {
        self.registration = None;
        self.armed = None;
        self.waker = None;
    }
    fn poll(
        &mut self,
        shared: &Arc<Shared>,
        deadline: Instant,
        now: Instant,
        context: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        // Extending a pending timeout need not touch the queue. Its old wake will rearm for the current desired deadline when polled.
        if self
            .armed
            .is_none_or(|armed| deadline < armed || now >= armed)
        {
            self.disarm();
            self.armed = Some(deadline);
        }
        if self.registration.is_none() {
            self.registration = Some(worker_timers::register(
                shared,
                deadline,
                context.waker().clone(),
            )?);
            self.waker = Some(context.waker().clone());
        }
        if !self.waker.as_ref().unwrap().will_wake(context.waker()) {
            let replacement = context.waker().clone();
            self.waker = Some(replacement.clone());
            self.registration.as_ref().unwrap().update(replacement);
        }
        Poll::Pending
    }
}
impl<S: Selector> Timers for NativeScheduler<S> {
    type Timer = NativeTimer;

    fn timer(&self) -> NativeTimer {
        NativeTimer {
            shared: Arc::clone(&self.tasks.shared),
            armed: None,
            registration: None,
            waker: None,
        }
    }

    async fn timer_wait_until(
        &self,
        timer: &mut NativeTimer,
        deadline: Instant,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        cancellation.check()?;
        let shared = &self.tasks.shared;
        if !Arc::ptr_eq(&timer.shared, shared) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "timer belongs to another scheduler",
            )
            .into());
        }
        let mut cancelled = pin!(cancellation.cancelled());
        let result = poll_fn(|context| {
            let now = self.now();
            if now >= deadline {
                return Poll::Ready(Ok(()));
            }
            if cancelled.as_mut().poll(context).is_ready() {
                return Poll::Ready(Err(base_scheduler::Cancelled.into()));
            }
            if shared.is_closed() {
                return Poll::Ready(Err(worker_timers::closed().into()));
            }
            timer
                .poll(shared, deadline, now, context)
                .map_err(Into::into)
        })
        .await;
        timer.disarm();
        result
    }
}
