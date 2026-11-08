// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Timer and scheduler workloads for base-scheduler. Each process measures one workload and writes one [`base_benchmark`] record.

mod after;
#[cfg(feature = "allocations")]
mod allocation;
pub mod options;
pub mod periodic;
pub mod scenario;
pub use after::After;
pub use options::{Backend, Benchmark, Mode, TimerOptions, TimerWorkload};

use base_benchmark::{
    CurrentProcess, Interval, Metric, ProcessRecord, ResourceSource, Sample, Unit,
};
use base_scheduler::{Cancellation, Spawn, Timers};
use std::{
    future::{Future, poll_fn},
    hint::black_box,
    io::{self, Write},
    pin::pin,
    process::ExitCode,
    sync::Arc,
    task::Poll,
    time::Duration,
};

/// The workload identity in every record.
pub const WORKLOAD: &str = "base-scheduler/timers";

/// Parse `arguments`, measure the workload, and write its record to `out`. Errors go to `errors`, and fail without a record.
pub fn command(
    arguments: impl IntoIterator<Item = String>,
    out: &mut impl Write,
    errors: &mut impl Write,
) -> ExitCode {
    command_with(arguments, CurrentProcess, out, errors)
}

/// [`command`], measuring resource usage with `source`.
pub fn command_with<R: ResourceSource>(
    arguments: impl IntoIterator<Item = String>,
    source: R,
    out: &mut impl Write,
    errors: &mut impl Write,
) -> ExitCode {
    match Benchmark::parse(arguments)
        .and_then(|benchmark| measure(&benchmark, source))
        .and_then(|record| record.emit(out))
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(errors, "base-scheduler-benchmark-timers: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Measure `benchmark`, with resource usage from `source`.
pub fn measure<R: ResourceSource>(benchmark: &Benchmark, source: R) -> io::Result<ProcessRecord> {
    let sample = match benchmark {
        Benchmark::Timer(options) => measure_timers(options, source),
        Benchmark::Scenario(options) => scenario::measure(options, source),
        Benchmark::Periodic(options) => periodic::measure(options, source),
    }?;
    Ok(ProcessRecord::new(WORKLOAD, benchmark.parameters()).with_sample(sample))
}

fn measure_timers<R: ResourceSource>(options: &TimerOptions, source: R) -> io::Result<Sample> {
    match options.backend {
        Backend::Tokio => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(options.workers)
                .enable_all()
                .build()
                .unwrap();
            let scheduler = Arc::new(base_scheduler_tokio::TokioScheduler::new(
                runtime.handle().clone(),
            ));
            let sample = runtime.block_on(run(scheduler.clone(), options, source));
            runtime.block_on(scheduler.shutdown());
            sample
        }
        Backend::Native => {
            let scheduler = Arc::new(
                base_scheduler_native::NativeScheduler::with_options(
                    base_scheduler_native::NativeOptions {
                        workers: options.workers,
                        timer_capacity: options.tasks.max(1024),
                        ..Default::default()
                    },
                )
                .unwrap(),
            );
            let sample = scheduler
                .block_on(run(scheduler.clone(), options, source))
                .map(|sample| with_rescues(sample, &scheduler));
            scheduler.block_on(scheduler.shutdown());
            sample
        }
        #[cfg(target_os = "linux")]
        Backend::IoUring => {
            let (selector, drivers) = base_scheduler_iouring::IoUringSelector::with_workers(
                Default::default(),
                options.workers,
            )
            .unwrap();
            let scheduler = Arc::new(
                base_scheduler_native::NativeScheduler::with_drivers(
                    selector,
                    drivers,
                    base_scheduler_native::NativeExecutorOptions {
                        workers: options.workers,
                        timer_capacity: options.tasks.max(1024),
                        ..Default::default()
                    },
                )
                .unwrap(),
            );
            let sample = scheduler
                .block_on(run(scheduler.clone(), options, source))
                .map(|sample| with_rescues(sample, &scheduler));
            scheduler.block_on(scheduler.shutdown());
            sample
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

/// One task's share of a timer workload: its completed operation attempts, the overshoot of each actual expiry in nanoseconds, and how many refreshes expired while being polled.
pub async fn workload<S: Timers>(
    scheduler: &S,
    mode: Mode,
    workload: TimerWorkload,
    iterations: usize,
    delay: Duration,
) -> (usize, Vec<u64>, usize) {
    use TimerWorkload::*;
    let never = Cancellation::new();
    let mut timer = scheduler.timer();
    let mut after = After::new(scheduler.timer());
    let mut overshoot = Vec::new();
    let mut refresh_expirations = 0;
    let expiring = matches!(workload, Expire | Cycle);
    if expiring {
        overshoot.reserve(iterations);
    }
    let fixed = scheduler.now() + Duration::from_secs(60);
    for iteration in 0..iterations {
        // Thirty seconds prevents accidental expiry in the refresh microbenchmarks. A changing deadline models refreshing a connection idle timeout.
        let deadline = match workload {
            Fixed => fixed,
            Earlier | EarlierYield => fixed - Duration::from_nanos(iteration as u64),
            _ if expiring => scheduler.now() + delay,
            _ => scheduler.now() + Duration::from_secs(30),
        };
        if workload == Expire {
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
            let count = if workload == Cycle { 32 } else { 1 };
            for _ in 0..count {
                let refreshed = if workload == Cycle {
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
                if matches!(workload, EarlierYield | Yield | Cycle) {
                    futures_lite::future::yield_now().await;
                }
            }
            if workload == Cycle {
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
        iterations * if workload == Cycle { 33 } else { 1 },
        overshoot,
        refresh_expirations,
    )
}

/// Run a timer workload in `options.tasks` tasks, measuring from their admission to their last join.
pub async fn run<S, R>(scheduler: Arc<S>, options: &TimerOptions, source: R) -> io::Result<Sample>
where
    S: Timers + Spawn + Send + Sync + 'static,
    R: ResourceSource,
{
    let TimerOptions {
        workload: workload_name,
        mode,
        iterations,
        delay,
        ..
    } = *options;
    #[cfg(feature = "allocations")]
    if workload_name == TimerWorkload::Alloc {
        let instance = scheduler.clone();
        return Ok(scheduler
            .spawn(async move { allocation::measure(&*instance, mode, iterations).await })
            .unwrap()
            .await
            .unwrap());
    }
    let interval = Interval::start_with(source)?;
    let mut handles = Vec::new();
    for _ in 0..options.tasks {
        let instance = scheduler.clone();
        handles.push(
            scheduler
                .spawn(
                    async move { workload(&*instance, mode, workload_name, iterations, delay).await },
                )
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
    let sample = interval.finish(operations as u64)?.with(Metric::total(
        "refresh_expirations",
        Unit::Count,
        refresh_expirations as u64,
    ));
    Ok(with_overshoot(sample, overshoot))
}

/// Add the 50th and 99th percentiles and the maximum of `overshoot`, in nanoseconds, when there is any.
///
/// They are order statistics of the measured expiries, reported as quantiles: as they are, never divided by operations.
pub fn with_overshoot(sample: Sample, mut overshoot: Vec<u64>) -> Sample {
    overshoot.sort_unstable();
    [
        ("overshoot_p50", 0.5),
        ("overshoot_p99", 0.99),
        ("overshoot_max", 1.0),
    ]
    .into_iter()
    .filter_map(|(name, quantile)| {
        let index = ((overshoot.len().checked_sub(1)?) as f64 * quantile) as usize;
        Some(
            Metric::quantile(name, Unit::Nanoseconds, quantile, overshoot[index] as f64)
                .lower_is_better(),
        )
    })
    .fold(sample, Sample::with)
}

/// Add a Native scheduler's stall rescue counts with the `diagnostics` feature. They cover the scheduler's whole life, not only the measured operations.
pub fn with_rescues<S: base_scheduler_native::Selector>(
    sample: Sample,
    scheduler: &base_scheduler_native::NativeScheduler<S>,
) -> Sample {
    #[cfg(feature = "diagnostics")]
    {
        let statistics = scheduler.rescue_statistics();
        [
            ("rescues_stuck_polls", statistics.stuck_polls),
            ("rescues_long_batches", statistics.long_batches),
            ("rescues_leaving_idle", statistics.leaving_idle),
            ("rescued_timers", statistics.timers),
        ]
        .into_iter()
        .fold(sample, |sample, (name, count)| {
            sample.with(
                Metric::total(name, Unit::Count, count as u64)
                    .with_phase(base_benchmark::Phase::Process),
            )
        })
    }
    #[cfg(not(feature = "diagnostics"))]
    {
        let _ = scheduler;
        sample
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

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod tests;
