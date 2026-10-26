// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

mod after;
mod allocation;
pub use after::After;

use base_scheduler::{Cancellation, Spawn, Timers};
use std::{
    future::{Future, poll_fn},
    hint::black_box,
    pin::pin,
    sync::Arc,
    task::Poll,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
pub enum Mode {
    Sleep,
    Reuse,
    After,
}
impl Mode {
    pub fn parse(mode: &str) -> Self {
        match mode {
            "sleep" => Self::Sleep,
            "reuse" => Self::Reuse,
            "after" => Self::After,
            _ => panic!("unknown mode"),
        }
    }
}

pub async fn pending_once(future: impl Future) {
    let mut future = pin!(future);
    poll_fn(|cx| {
        assert!(
            future.as_mut().poll(cx).is_pending(),
            "refresh timer unexpectedly expired"
        );
        Poll::Ready(())
    })
    .await;
}

pub async fn workload<S: Timers>(
    scheduler: &S,
    mode: Mode,
    workload: &str,
    iterations: usize,
    delay: Duration,
) -> (usize, Vec<u64>, usize) {
    let never = Cancellation::new();
    let mut timer = scheduler.timer();
    let mut after = After::new(scheduler.timer());
    let mut overshoot = Vec::new();
    let mut refresh_expirations = 0;
    if workload == "expire" || workload == "cycle" {
        overshoot.reserve(iterations);
    }
    let fixed = scheduler.now() + Duration::from_secs(60);
    for iteration in 0..iterations {
        // Thirty seconds prevents accidental expiry in the refresh microbenchmarks. A changing deadline models refreshing a connection idle timeout.
        let deadline = if workload == "fixed" {
            fixed
        } else if workload.starts_with("earlier") {
            fixed - Duration::from_nanos(iteration as u64)
        } else {
            scheduler.now()
                + if workload == "expire" || workload == "cycle" {
                    delay
                } else {
                    Duration::from_secs(30)
                }
        };
        if workload == "expire" {
            match mode {
                Mode::Sleep => scheduler.sleep_until(deadline, &never).await.unwrap(),
                Mode::Reuse => scheduler
                    .timer_wait_until(&mut timer, deadline, &never)
                    .await
                    .unwrap(),
                Mode::After => after.wait(scheduler, deadline, &never).await.unwrap(),
            }
            overshoot.push(
                scheduler
                    .now()
                    .saturating_duration_since(deadline)
                    .as_nanos() as u64,
            );
        } else {
            let count = if workload == "cycle" { 32 } else { 1 };
            for _ in 0..count {
                let refreshed = if workload == "cycle" {
                    scheduler.now() + delay
                } else {
                    deadline
                };
                let expired = match mode {
                    Mode::Sleep => probe(scheduler.sleep_until(refreshed, &never)).await,
                    Mode::Reuse => {
                        probe(scheduler.timer_wait_until(&mut timer, refreshed, &never)).await
                    }
                    Mode::After => probe(after.wait(scheduler, refreshed, &never)).await,
                };
                refresh_expirations += usize::from(expired);
                if !matches!(workload, "refresh" | "fixed" | "earlier") {
                    futures_lite::future::yield_now().await;
                }
            }
            if workload == "cycle" {
                let last = scheduler.now() + delay;
                match mode {
                    Mode::Sleep => scheduler.sleep_until(last, &never).await.unwrap(),
                    Mode::Reuse => scheduler
                        .timer_wait_until(&mut timer, last, &never)
                        .await
                        .unwrap(),
                    Mode::After => after.wait(scheduler, last, &never).await.unwrap(),
                }
                overshoot.push(scheduler.now().saturating_duration_since(last).as_nanos() as u64);
            }
        }
        black_box(iteration);
    }
    (
        iterations * if workload == "cycle" { 33 } else { 1 },
        overshoot,
        refresh_expirations,
    )
}

pub async fn run<S: Timers + Spawn + Send + Sync + 'static>(
    scheduler: Arc<S>,
    mode: Mode,
    workload_name: String,
    tasks: usize,
    iterations: usize,
    delay: Duration,
) -> serde_json::Value {
    if workload_name == "alloc" {
        let instance = scheduler.clone();
        return scheduler
            .spawn(async move { allocation::measure(&*instance, mode, iterations).await })
            .unwrap()
            .await
            .unwrap();
    }
    let cpu_start = cpu_time();
    let start = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..tasks {
        let instance = scheduler.clone();
        let workload_name = workload_name.clone();
        handles.push(
            scheduler
                .spawn(async move {
                    workload(&*instance, mode, &workload_name, iterations, delay).await
                })
                .unwrap(),
        );
    }
    let mut operations = 0;
    let mut overshoot = Vec::new();
    let mut refresh_expirations = 0;
    for handle in handles {
        let (count, values, expired) = handle.await.unwrap();
        operations += count;
        refresh_expirations += expired;
        overshoot.extend(values);
    }
    let elapsed = start.elapsed().as_secs_f64();
    let cpu_seconds = cpu_time() - cpu_start;
    overshoot.sort_unstable();
    let percentile = |p: f64| {
        overshoot
            .get(((overshoot.len().saturating_sub(1) as f64) * p) as usize)
            .copied()
    };
    serde_json::json!({"seconds": elapsed, "refresh_expirations": refresh_expirations, "cpu_seconds": cpu_seconds, "cpu_ns_per_operation": cpu_seconds * 1e9 / operations as f64, "operations": operations, "ops_per_second": operations as f64 / elapsed, "ns_per_operation": elapsed * 1e9 / operations as f64, "overshoot_p50_ns": percentile(0.5), "overshoot_p99_ns": percentile(0.99)})
}

#[cfg(test)]
mod tests;

fn cpu_time() -> f64 {
    #[cfg(unix)]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        // SAFETY: getrusage initializes the output on success; the pointer refers to writable storage of the required size.
        assert_eq!(
            unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) },
            0
        );
        // SAFETY: The successful getrusage call initialized this value.
        let usage = unsafe { usage.assume_init() };
        (usage.ru_utime.tv_sec + usage.ru_stime.tv_sec) as f64
            + (usage.ru_utime.tv_usec + usage.ru_stime.tv_usec) as f64 / 1e6
    }
    #[cfg(not(unix))]
    {
        0.0
    }
}

async fn probe(future: impl Future<Output = base_scheduler::OperationResult<()>>) -> bool {
    let mut future = pin!(future);
    poll_fn(|context| {
        Poll::Ready(match future.as_mut().poll(context) {
            Poll::Ready(result) => {
                result.unwrap();
                true
            }
            Poll::Pending => false,
        })
    })
    .await
}
