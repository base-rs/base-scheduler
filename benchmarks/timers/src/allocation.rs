// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{After, Mode};
use core_scheduler::{Cancellation, TimerClock};
use std::{
    future::{Future, poll_fn},
    pin::pin,
    task::{Context, Poll},
    time::Duration,
};

fn pending(future: impl Future, context: &mut Context<'_>) {
    assert!(pin!(future).as_mut().poll(context).is_pending());
}

pub async fn measure<S: TimerClock>(
    scheduler: &S,
    mode: Mode,
    iterations: usize,
) -> serde_json::Value {
    let token = Cancellation::new();
    let mut timer = S::Timer::default();
    let mut after = After::<S::Timer>::default();
    let waker = poll_fn(|context| Poll::Ready(context.waker().clone())).await;
    let mut context = Context::from_waker(&waker);
    let mut step = || {
        let deadline = scheduler.now() + Duration::from_secs(30);
        match mode {
            Mode::Sleep => pending(scheduler.sleep_until(deadline, &token), &mut context),
            Mode::Reuse => pending(
                scheduler.timer_wait(&mut timer, deadline, &token),
                &mut context,
            ),
            Mode::After => pending(after.wait(scheduler, deadline, &token), &mut context),
        }
    };
    // Warm the resource, task-waker and queue capacity before counting per-operation work.
    for _ in 0..2048 {
        step();
    }
    let measured = allocation_counter::measure(|| {
        for _ in 0..iterations {
            step();
        }
    });
    serde_json::json!({"allocations": measured.count_total, "allocated_bytes": measured.bytes_total, "operations": iterations, "timer_bytes": std::mem::size_of::<S::Timer>(), "after_bytes": std::mem::size_of::<After<S::Timer>>(), "count_scope": "calling worker only; background-thread allocations excluded"})
}
