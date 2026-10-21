// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{Cancellation, Clock, OperationResult};
use std::{future::Future, time::Instant};

/// Experimental reusable timer capability for the timer benchmark. This is not a settled public contract.
///
/// A default resource is unbound. The first non-cancelled wait admitted by a clock binds it, including an already elapsed deadline; another incompatible clock is rejected. Waiting always receives its scheduler and cancellation explicitly. Only one waiter can borrow the resource. Dropping a wait retains its registration, which may cause a later wake of the previous task. Dropping the resource retires it; forgetting it may leak registration capacity but cannot retain borrowed application memory. Cancellation observed after admission disarms its registration; pre-cancellation leaves existing state unchanged. Deadline replacement happens on first poll, including an earlier replacement.
pub trait TimerClock: Clock {
    type Timer: Default + Send;

    fn timer_wait(
        &self,
        timer: &mut Self::Timer,
        deadline: Instant,
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<()>> + Send;
}
