// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Periodic timers: many tasks each sleep for a fixed delay again and again, and the overshoot of every expiry inside a measurement window is recorded. This is the worker-local timer diagnostic of the stall rescue design.

use crate::{cpu_time, quantile, rescues};
use base_scheduler::{Cancellation, Clock, Spawn};
use base_scheduler_native::{NativeScheduler, Selector};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

const USAGE: &str =
    "periodic BACKEND WORKERS TASKS DELAY_US [unaligned|aligned] [MEASURE_MS] [WORK]";

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

/// Runs `periodic BACKEND WORKERS TASKS DELAY_US [unaligned|aligned] [MEASURE_MS] [WORK]` and returns its record, with overshoot in nanoseconds. `BACKEND` is `native`, or `iouring` on Linux. The window measures for `MEASURE_MS` (3,000 by default) after a warmup of a third of that.
pub fn execute(args: &[String]) -> serde_json::Value {
    let backend = args.first().expect(USAGE).as_str();
    let workers: usize = args.get(1).expect(USAGE).parse().unwrap();
    let tasks: usize = args.get(2).expect(USAGE).parse().unwrap();
    let delay = Duration::from_micros(args.get(3).expect(USAGE).parse().unwrap());
    assert!(!delay.is_zero(), "DELAY_US must be positive");
    let alignment = args.get(4).map_or("unaligned", String::as_str);
    let aligned = match alignment {
        "unaligned" => false,
        "aligned" => true,
        _ => panic!("unknown alignment"),
    };
    let measure = Duration::from_millis(args.get(5).map_or(3000, |value| value.parse().unwrap()));
    let work: u64 = args.get(6).map_or(0, |value| value.parse().unwrap());
    let start = Instant::now() + measure / 3;
    let window = Arc::new(Window {
        start,
        end: start + measure,
        delay,
        aligned,
        work,
    });
    let capacity = tasks * 2 + 256;
    let cpu_start = cpu_time();
    let (mut overshoot, rescues) = match backend {
        "native" => run(
            NativeScheduler::with_options(base_scheduler_native::NativeOptions {
                workers,
                timer_capacity: capacity,
                ..Default::default()
            })
            .unwrap(),
            tasks,
            window,
        ),
        #[cfg(target_os = "linux")]
        "iouring" => {
            let (selector, drivers) =
                base_scheduler_iouring::IoUringSelector::with_workers(Default::default(), workers)
                    .unwrap();
            run(
                NativeScheduler::with_drivers(
                    selector,
                    drivers,
                    base_scheduler_native::NativeExecutorOptions {
                        workers,
                        timer_capacity: capacity,
                        ..Default::default()
                    },
                )
                .unwrap(),
                tasks,
                window,
            )
        }
        _ => panic!("unknown backend"),
    };
    let cpu_seconds = cpu_time() - cpu_start;
    overshoot.sort_unstable();
    serde_json::json!({
        "backend": backend,
        "workers": workers,
        "tasks": tasks,
        "delay_us": delay.as_micros() as u64,
        "alignment": alignment,
        "work": work,
        "seconds": measure.as_secs_f64(),
        "samples": overshoot.len(),
        "expiries_per_second": overshoot.len() as f64 / measure.as_secs_f64(),
        "overshoot_p50_ns": quantile(&overshoot, 0.5),
        "overshoot_p99_ns": quantile(&overshoot, 0.99),
        "overshoot_max_ns": quantile(&overshoot, 1.0),
        "cpu_seconds": cpu_seconds,
        "rescues": rescues,
    })
}

// Runs every task to the end of the window, and returns the overshoot of each expiry inside it in nanoseconds, with the rescue counts.
fn run<S: Selector + 'static>(
    scheduler: NativeScheduler<S>,
    tasks: usize,
    window: Arc<Window>,
) -> (Vec<u64>, serde_json::Value) {
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
    let overshoot = scheduler.block_on(async {
        let mut overshoot = Vec::new();
        for handle in handles {
            overshoot.extend(handle.await.unwrap());
        }
        overshoot
    });
    let rescues = rescues(&scheduler);
    scheduler.block_on(scheduler.shutdown());
    (overshoot, rescues)
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
