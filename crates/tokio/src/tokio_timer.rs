// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::TokioScheduler;
use core_scheduler::{Cancellation, Clock, OperationResult, TimerClock};
use std::{
    future::{Future, poll_fn},
    io,
    pin::{Pin, pin},
    task::Poll,
    time::Instant,
};

/// Experimental retained Tokio sleep. Its pinning allocation is made on first use, then reused.
#[derive(Default)]
pub struct TokioTimer {
    sleep: Option<Pin<Box<tokio::time::Sleep>>>,
    runtime: Option<tokio::runtime::Id>,
}
impl TimerClock for TokioScheduler {
    type Timer = TokioTimer;

    async fn timer_wait(
        &self,
        timer: &mut TokioTimer,
        deadline: Instant,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        cancellation.check()?;
        self.check_open()?;
        if timer
            .runtime
            .is_some_and(|runtime| runtime != self.runtime.id())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "timer belongs to another runtime",
            )
            .into());
        }
        timer.runtime = Some(self.runtime.id());
        if self.now() >= deadline {
            timer.sleep = None;
            return Ok(());
        }
        let deadline = tokio::time::Instant::from_std(deadline);
        {
            let _scope = self.runtime.enter();
            if let Some(sleep) = timer.sleep.as_mut() {
                if sleep.deadline() != deadline || sleep.is_elapsed() {
                    sleep.as_mut().reset(deadline);
                }
            } else {
                timer.sleep = Some(Box::pin(tokio::time::sleep_until(deadline)));
            }
        }
        let mut cancelled = pin!(cancellation.cancelled());
        let result = poll_fn(|context| {
            let _scope = self.runtime.enter();
            if timer
                .sleep
                .as_mut()
                .unwrap()
                .as_mut()
                .poll(context)
                .is_ready()
            {
                return Poll::Ready(Ok(()));
            }
            if cancelled.as_mut().poll(context).is_ready() {
                return Poll::Ready(Err(core_scheduler::Cancelled.into()));
            }
            Poll::Pending
        })
        .await;
        if result.is_err() {
            timer.sleep = None;
        }
        result
    }
}
