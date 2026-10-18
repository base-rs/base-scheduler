// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::TokioScheduler;
use protocol_scheduler::{Cancellation, Clock, OperationResult};
use std::{
    future::{Future, poll_fn},
    pin::pin,
    task::Poll,
    time::Instant,
};

impl Clock for TokioScheduler {
    fn now(&self) -> Instant {
        let _scope = self.runtime.enter();
        tokio::time::Instant::now().into_std()
    }
    async fn sleep_until(
        &self,
        deadline: Instant,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        cancellation.check()?;
        self.check_open()?;
        if self.now() >= deadline {
            return Ok(());
        }
        let sleep = {
            let _scope = self.runtime.enter();
            tokio::time::sleep_until(tokio::time::Instant::from_std(deadline))
        };
        #[cfg(test)]
        let _registration = tests::Registration::new(&self.timers);
        let mut sleep = pin!(sleep);
        let mut cancelled = pin!(cancellation.cancelled());
        let mut closed = pin!(self.closed.cancelled());
        poll_fn(|context| {
            let _scope = self.runtime.enter();
            if sleep.as_mut().poll(context).is_ready() {
                return Poll::Ready(Ok(()));
            }
            if cancelled.as_mut().poll(context).is_ready() {
                return Poll::Ready(Err(protocol_scheduler::Cancelled.into()));
            }
            if closed.as_mut().poll(context).is_ready() {
                return Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "clock is closed",
                )
                .into()));
            }
            Poll::Pending
        })
        .await
    }
}

#[cfg(test)]
mod tests;
