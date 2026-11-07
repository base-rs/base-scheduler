// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Periodic timers: many tasks each sleep for a fixed delay again and again, and the overshoot of every expiry inside a measurement window is recorded. This is the worker-local timer diagnostic of the stall rescue design.

use crate::options::{Alignment, NativeBackend, PeriodicOptions};
use crate::{with_overshoot, with_rescues};
use base_benchmark::{Interval, ResourceSource, Sample};
use base_scheduler::{Cancellation, Clock, Spawn};
use base_scheduler_native::{NativeScheduler, Selector};
use std::{
    io,
    sync::Arc,
    time::{Duration, Instant},
};

struct Window {
    start: Instant,
    end: Instant,
    delay: Duration,
    aligned: bool,
    work: u64,
}

impl Window {
    // The next deadline: one delay from now, or with `aligned`, the next multiple of the delay, so that every task's deadlines fall due together.
    fn target(&self) -> Instant {
        let now = Instant::now();
        if !self.aligned {
            return now + self.delay;
        }
        // Any fixed epoch would do; one second before the window keeps the grid independent of when tasks start.
        let epoch = self.start - Duration::from_secs(1);
        let ticks = now.duration_since(epoch).as_nanos() / self.delay.as_nanos() + 1;
        epoch + self.delay * u32::try_from(ticks).unwrap()
    }

    // Simulated work after each expiry, `work` multiply-add steps.
    fn work(&self) {
        let mut value = 1u64;
        for index in 0..self.work {
            value =
                std::hint::black_box(value.wrapping_mul(6364136223846793005).wrapping_add(index));
        }
        std::hint::black_box(value);
    }
}

/// Measure the window: its CPU time and wall time, with every expiry inside it as one operation and the overshoot of each. The window follows a warmup of a third of its length.
pub fn measure<R: ResourceSource>(options: &PeriodicOptions, source: R) -> io::Result<Sample> {
    let start = Instant::now() + options.measure / 3;
    let window = Arc::new(Window {
        start,
        end: start + options.measure,
        delay: options.delay,
        aligned: options.alignment == Alignment::Aligned,
        work: options.work,
    });
    let capacity = options.tasks * 2 + 256;
    match options.backend {
        NativeBackend::Native => run(
            NativeScheduler::with_options(base_scheduler_native::NativeOptions {
                workers: options.workers,
                timer_capacity: capacity,
                ..Default::default()
            })
            .unwrap(),
            options.tasks,
            window,
            source,
        ),
        #[cfg(target_os = "linux")]
        NativeBackend::IoUring => {
            let (selector, drivers) = base_scheduler_iouring::IoUringSelector::with_workers(
                Default::default(),
                options.workers,
            )
            .unwrap();
            run(
                NativeScheduler::with_drivers(
                    selector,
                    drivers,
                    base_scheduler_native::NativeExecutorOptions {
                        workers: options.workers,
                        timer_capacity: capacity,
                        ..Default::default()
                    },
                )
                .unwrap(),
                options.tasks,
                window,
                source,
            )
        }
    }
}

// Runs every task to the end of the window while the root thread, which runs no tasks, measures the window.
fn run<S: Selector + 'static, R: ResourceSource>(
    scheduler: NativeScheduler<S>,
    tasks: usize,
    window: Arc<Window>,
    source: R,
) -> io::Result<Sample> {
    let scheduler = Arc::new(scheduler);
    let handles: Vec<_> = (0..tasks)
        .map(|_| {
            let task_scheduler = Arc::clone(&scheduler);
            let window = Arc::clone(&window);
            scheduler
                .spawn(async move { expiries(task_scheduler.as_ref(), &window).await })
                .unwrap()
        })
        .collect();
    let sample = measure_window(&window, source);
    let overshoot = scheduler.block_on(async {
        let mut overshoot = Vec::new();
        for handle in handles {
            overshoot.extend(handle.await.unwrap());
        }
        overshoot
    });
    let sample = sample.and_then(|mut sample| {
        if overshoot.is_empty() {
            return Err(io::Error::other(
                "no timer expired inside the measurement window",
            ));
        }
        sample.operations = overshoot.len() as u64;
        Ok(with_rescues(with_overshoot(sample, overshoot), &scheduler))
    });
    scheduler.block_on(scheduler.shutdown());
    sample
}

// The resource usage between the window's start and end. Its operations are the window's expiries, counted once the tasks finish.
fn measure_window<R: ResourceSource>(window: &Window, source: R) -> io::Result<Sample> {
    std::thread::sleep(window.start.saturating_duration_since(Instant::now()));
    let interval = Interval::start_with(source)?;
    std::thread::sleep(window.end.saturating_duration_since(Instant::now()));
    interval.finish(1)
}

async fn expiries<S: Clock>(scheduler: &S, window: &Window) -> Vec<u64> {
    let never = Cancellation::never();
    let window_length = window.end.duration_since(window.start);
    // Reserve for the window's expiries up front, so that recording rarely allocates, but not unboundedly for very short delays.
    let expected = (window_length.as_nanos() / window.delay.as_nanos()) as usize + 1;
    let mut overshoot = Vec::with_capacity(expected.min(1 << 16));
    while Instant::now() < window.end {
        let target = window.target();
        scheduler.sleep_until(target, &never).await.unwrap();
        let now = Instant::now();
        assert!(now >= target, "timer fired early");
        if target >= window.start && now < window.end {
            overshoot.push(now.duration_since(target).as_nanos() as u64);
        }
        window.work();
    }
    overshoot
}

#[cfg(test)]
mod tests;
