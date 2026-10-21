// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::{Cancellation, OperationResult, TimerClock};
use std::time::Instant;

/// Experimental timeout state. Later deadlines retain the existing wake; earlier deadlines replace it immediately. No scheduler is stored here.
pub struct After<T> {
    timer: T,
    armed: Option<Instant>,
}
impl<T: Default> Default for After<T> {
    fn default() -> Self {
        Self {
            timer: T::default(),
            armed: None,
        }
    }
}
impl<T> After<T> {
    pub async fn wait<S: TimerClock<Timer = T>>(
        &mut self,
        scheduler: &S,
        deadline: Instant,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        cancellation.check()?;
        loop {
            // Preserve an earlier wake when extending an idle timeout. It may wake early once; only then touch the backend queue again.
            let armed = self.armed.map_or(deadline, |armed| armed.min(deadline));
            self.armed = Some(armed);
            let result = scheduler
                .timer_wait(&mut self.timer, armed, cancellation)
                .await;
            self.armed = None;
            result?;
            if scheduler.now() >= deadline {
                return Ok(());
            }
        }
    }
}
