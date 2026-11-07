// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Native scheduler scenarios driven from the root thread: root and task sleeps, spawning with and without a pending root timer, idle and pulse CPU, and timers behind a stalled worker.

use crate::options::{ScenarioOptions, ScenarioWorkload};
use crate::{with_overshoot, with_rescues};
use base_benchmark::{Interval, ResourceSource, Sample};
use base_scheduler::{Cancellation, Clock, Spawn};
use base_scheduler_native::{NativeOptions, NativeScheduler};
use futures_lite::future::{block_on, poll_once, yield_now};
use std::{
    io,
    sync::Arc,
    time::{Duration, Instant},
};

/// Measure a scenario over its iterations, with the overshoot of each measured sleep. Every duration is a multiple of the unit: sleeps last one unit, a pulse 1,000 units, and a stall blocks a worker for 200 units while root sleeps of 10 units are measured.
pub fn measure<R: ResourceSource>(options: &ScenarioOptions, source: R) -> io::Result<Sample> {
    let scheduler = Arc::new(
        NativeScheduler::with_options(NativeOptions {
            workers: options.workers,
            ..Default::default()
        })
        .unwrap(),
    );
    // Let the workers start and go idle before measuring.
    std::thread::sleep(options.unit * 50);
    let sample = Interval::start_with(source).and_then(|interval| {
        let overshoot = run(
            &scheduler,
            options.workload,
            options.workers,
            options.iterations,
            options.unit,
        );
        let sample = interval.finish(options.iterations as u64)?;
        Ok(with_rescues(with_overshoot(sample, overshoot), &scheduler))
    });
    block_on(scheduler.shutdown());
    sample
}

// Returns the overshoot of each measured sleep in nanoseconds.
fn run(
    scheduler: &Arc<NativeScheduler>,
    workload: ScenarioWorkload,
    workers: usize,
    iterations: usize,
    unit: Duration,
) -> Vec<u64> {
    let never = Cancellation::never();
    let mut overshoot = Vec::new();
    match workload {
        // The root thread sleeps one unit repeatedly. Root registrations go to each worker in turn.
        ScenarioWorkload::RootSleep => {
            for _ in 0..iterations {
                overshoot.push(root_sleep(scheduler, unit));
            }
        }
        // The root thread spawns and joins an empty task, optionally with a distant root sleep pending throughout.
        ScenarioWorkload::Spawn | ScenarioWorkload::SpawnPending => {
            let mut pending = Box::pin(scheduler.sleep(Duration::from_secs(3600), &never));
            if workload == ScenarioWorkload::SpawnPending {
                assert!(block_on(poll_once(&mut pending)).is_none());
            }
            for _ in 0..iterations {
                block_on(scheduler.spawn(async {}).unwrap()).unwrap();
            }
        }
        // Four tasks per worker each sleep one unit `iterations` times, optionally with a distant root sleep pending throughout.
        ScenarioWorkload::TaskSleep | ScenarioWorkload::TaskSleepPending => {
            let mut pending = Box::pin(scheduler.sleep(Duration::from_secs(3600), &never));
            if workload == ScenarioWorkload::TaskSleepPending {
                assert!(block_on(poll_once(&mut pending)).is_none());
            }
            let tasks: Vec<_> = (0..workers * 4)
                .map(|_| {
                    let task_scheduler = Arc::clone(scheduler);
                    scheduler
                        .spawn(async move {
                            let never = Cancellation::never();
                            let mut overshoot = Vec::with_capacity(iterations);
                            for _ in 0..iterations {
                                let deadline = task_scheduler.now() + unit;
                                task_scheduler.sleep_until(deadline, &never).await.unwrap();
                                overshoot.push(since(deadline));
                            }
                            overshoot
                        })
                        .unwrap()
                })
                .collect();
            for task in tasks {
                overshoot.extend(block_on(task).unwrap());
            }
        }
        // No load: process CPU while every worker is idle for `iterations` units.
        ScenarioWorkload::Idle => std::thread::sleep(unit * u32::try_from(iterations).unwrap()),
        // A task woken by a timer every 1,000 units, `iterations` times: one wake a second by default.
        ScenarioWorkload::Pulse => {
            let task_scheduler = Arc::clone(scheduler);
            let pulse = scheduler
                .spawn(async move {
                    let never = Cancellation::never();
                    for _ in 0..iterations {
                        task_scheduler.sleep(unit * 1000, &never).await.unwrap();
                    }
                })
                .unwrap();
            block_on(pulse).unwrap();
        }
        // A task blocks its worker inside one poll for 200 units while the root thread measures eight sleeps of 10 units. Root registrations go to each worker in turn, so some land on the blocked one.
        ScenarioWorkload::Stall => {
            for _ in 0..iterations {
                let blocker = scheduler
                    .spawn(async move { std::thread::sleep(unit * 200) })
                    .unwrap();
                std::thread::sleep(unit * 5);
                for _ in 0..8 {
                    overshoot.push(root_sleep(scheduler, unit * 10));
                }
                block_on(blocker).unwrap();
            }
        }
        // Sixteen tasks per worker yield `iterations` times each, with no timers: the scheduler loop alone.
        ScenarioWorkload::TaskYield => {
            let tasks: Vec<_> = (0..workers * 16)
                .map(|_| {
                    scheduler
                        .spawn(async move {
                            for _ in 0..iterations {
                                yield_now().await;
                            }
                        })
                        .unwrap()
                })
                .collect();
            for task in tasks {
                block_on(task).unwrap();
            }
        }
    }
    overshoot
}

fn root_sleep(scheduler: &NativeScheduler, delay: Duration) -> u64 {
    let deadline = Instant::now() + delay;
    block_on(scheduler.sleep_until(deadline, &Cancellation::never())).unwrap();
    since(deadline)
}

fn since(deadline: Instant) -> u64 {
    Instant::now()
        .saturating_duration_since(deadline)
        .as_nanos() as u64
}

#[cfg(test)]
mod tests;
