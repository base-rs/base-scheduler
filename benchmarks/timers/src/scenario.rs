// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Native scheduler scenarios driven from the root thread: root and task sleeps, spawning with and without a pending root timer, idle and pulse CPU, and timers behind a stalled worker.

use crate::{cpu_time, quantile, rescues};
use base_scheduler::{Cancellation, Clock, Spawn};
use base_scheduler_native::{NativeOptions, NativeScheduler};
use futures_lite::future::{block_on, poll_once, yield_now};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

const USAGE: &str = "scenario WORKLOAD WORKERS ITERATIONS [UNIT_US]";

/// Runs `scenario WORKLOAD WORKERS ITERATIONS [UNIT_US]` and returns its record. Every duration is a multiple of the unit, one millisecond by default: sleeps last one unit, a pulse 1,000 units, and a stall blocks a worker for 200 units while root sleeps of 10 units are measured.
pub fn execute(args: &[String]) -> serde_json::Value {
    let workload = args.first().expect(USAGE).as_str();
    let workers: usize = args.get(1).expect(USAGE).parse().unwrap();
    let iterations: usize = args.get(2).expect(USAGE).parse().unwrap();
    let unit_us: u64 = args.get(3).map_or(1000, |unit| unit.parse().unwrap());
    let unit = Duration::from_micros(unit_us);
    let scheduler = Arc::new(
        NativeScheduler::with_options(NativeOptions {
            workers,
            ..Default::default()
        })
        .unwrap(),
    );
    // Let the workers start and go idle before measuring.
    std::thread::sleep(unit * 50);
    let cpu_start = cpu_time();
    let start = Instant::now();
    let mut lateness = run(&scheduler, workload, workers, iterations, unit);
    let elapsed = start.elapsed();
    let cpu_seconds = cpu_time() - cpu_start;
    let rescues = rescues(&scheduler);
    block_on(scheduler.shutdown());
    lateness.sort_unstable();
    serde_json::json!({
        "workload": workload,
        "workers": workers,
        "iterations": iterations,
        "unit_us": unit_us,
        "ns_per_op": elapsed.as_nanos() as f64 / iterations as f64,
        "cpu_ns_per_op": cpu_seconds * 1e9 / iterations as f64,
        "samples": lateness.len(),
        "p50": quantile(&lateness, 0.5),
        "p99": quantile(&lateness, 0.99),
        "max": quantile(&lateness, 1.0),
        "rescues": rescues,
    })
}

// Returns the lateness of each measured sleep in nanoseconds.
fn run(
    scheduler: &Arc<NativeScheduler>,
    workload: &str,
    workers: usize,
    iterations: usize,
    unit: Duration,
) -> Vec<u64> {
    let never = Cancellation::never();
    let mut lateness = Vec::new();
    match workload {
        // The root thread sleeps one unit repeatedly. Root registrations go to each worker in turn.
        "root-sleep" => {
            for _ in 0..iterations {
                lateness.push(root_sleep(scheduler, unit));
            }
        }
        // The root thread spawns and joins an empty task, optionally with a distant root sleep pending throughout.
        "spawn" | "spawn-pending" => {
            let mut pending = Box::pin(scheduler.sleep(Duration::from_secs(3600), &never));
            if workload == "spawn-pending" {
                assert!(block_on(poll_once(&mut pending)).is_none());
            }
            for _ in 0..iterations {
                block_on(scheduler.spawn(async {}).unwrap()).unwrap();
            }
        }
        // Four tasks per worker each sleep one unit `iterations` times, optionally with a distant root sleep pending throughout.
        "task-sleep" | "task-sleep-pending" => {
            let mut pending = Box::pin(scheduler.sleep(Duration::from_secs(3600), &never));
            if workload == "task-sleep-pending" {
                assert!(block_on(poll_once(&mut pending)).is_none());
            }
            let tasks: Vec<_> = (0..workers * 4)
                .map(|_| {
                    let task_scheduler = Arc::clone(scheduler);
                    scheduler
                        .spawn(async move {
                            let never = Cancellation::never();
                            let mut lateness = Vec::with_capacity(iterations);
                            for _ in 0..iterations {
                                let deadline = task_scheduler.now() + unit;
                                task_scheduler.sleep_until(deadline, &never).await.unwrap();
                                lateness.push(since(deadline));
                            }
                            lateness
                        })
                        .unwrap()
                })
                .collect();
            for task in tasks {
                lateness.extend(block_on(task).unwrap());
            }
        }
        // No load: process CPU while every worker is idle for `iterations` units.
        "idle" => std::thread::sleep(unit * u32::try_from(iterations).unwrap()),
        // A task woken by a timer every 1,000 units, `iterations` times: one wake a second by default.
        "pulse" => {
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
        "stall" => {
            for _ in 0..iterations {
                let blocker = scheduler
                    .spawn(async move { std::thread::sleep(unit * 200) })
                    .unwrap();
                std::thread::sleep(unit * 5);
                for _ in 0..8 {
                    lateness.push(root_sleep(scheduler, unit * 10));
                }
                block_on(blocker).unwrap();
            }
        }
        // Sixteen tasks per worker yield `iterations` times each, with no timers: the scheduler loop alone.
        "yield" => {
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
        _ => panic!("unknown workload"),
    }
    lateness
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
