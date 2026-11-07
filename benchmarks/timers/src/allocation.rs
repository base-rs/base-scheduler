// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{After, Mode};
use base_benchmark::{Metric, Sample, Unit};
use base_scheduler::{Cancellation, Timers};
use std::{
    future::{Future, poll_fn},
    pin::pin,
    task::{Context, Poll},
    time::Duration,
};

fn pending(future: impl Future, context: &mut Context<'_>) {
    assert!(pin!(future).as_mut().poll(context).is_pending());
}

/// Count the allocations on the calling worker during `iterations` warmed refreshes. Background-thread allocations and setup are excluded.
pub async fn measure<S: Timers>(scheduler: &S, mode: Mode, iterations: usize) -> Sample {
    let token = Cancellation::new();
    let mut timer = scheduler.timer();
    let mut after = After::new(scheduler.timer());
    let waker = poll_fn(|context| Poll::Ready(context.waker().clone())).await;
    let mut context = Context::from_waker(&waker);
    let mut step = || {
        let deadline = scheduler.now() + Duration::from_secs(30);
        match mode {
            Mode::Sleep => pending(scheduler.sleep_until(deadline, &token), &mut context),
            Mode::Reuse => pending(
                scheduler.timer_wait_until(&mut timer, deadline, &token),
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
    let size = |name, bytes: usize| Metric::mean(name, Unit::Bytes, bytes as f64);
    Sample::new(iterations as u64)
        .with(Metric::total("allocations", Unit::Count, measured.count_total).lower_is_better())
        .with(Metric::total("allocated_bytes", Unit::Bytes, measured.bytes_total).lower_is_better())
        .with(size("timer_size", std::mem::size_of::<S::Timer>()))
        .with(size("after_size", std::mem::size_of::<After<S::Timer>>()))
}
