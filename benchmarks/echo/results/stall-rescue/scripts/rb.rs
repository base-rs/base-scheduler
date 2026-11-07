// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use base_scheduler::{Cancellation, Clock, Spawn};
use base_scheduler_native::{NativeOptions, NativeScheduler};
use futures_lite::future::{block_on, poll_once};
use std::time::{Duration, Instant};

fn cpu() -> f64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    let t = |v: libc::timeval| v.tv_sec as f64 + v.tv_usec as f64 / 1e6;
    t(usage.ru_utime) + t(usage.ru_stime)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let workload = args[1].as_str();
    let workers: usize = args[2].parse().unwrap();
    let iterations: usize = args[3].parse().unwrap();
    let scheduler = std::sync::Arc::new(NativeScheduler::with_options(NativeOptions { workers, ..Default::default() }).unwrap());
    let never = Cancellation::never();
    std::thread::sleep(Duration::from_millis(50));
    let mut lateness = Vec::new();
    let c0 = cpu();
    let start = Instant::now();
    match workload {
        // Root thread sleeping 1 ms repeatedly.
        "root-sleep" => {
            for _ in 0..iterations {
                let deadline = Instant::now() + Duration::from_millis(1);
                block_on(scheduler.sleep_until(deadline, &never)).unwrap();
                lateness.push(Instant::now().duration_since(deadline).as_nanos() as f64);
            }
        }
        // Root spawns and joins a trivial task, optionally with a distant root timer pending.
        "spawn" | "spawn-pending" => {
            let mut pending = Box::pin(scheduler.sleep(Duration::from_secs(3600), &never));
            if workload == "spawn-pending" {
                assert!(block_on(poll_once(&mut pending)).is_none());
            }
            for _ in 0..iterations {
                block_on(scheduler.spawn(async {}).unwrap()).unwrap();
            }
        }
        // Tasks repeatedly sleep 1 ms on workers, optionally with a distant root timer pending.
        "task-sleep" | "task-sleep-pending" => {
            let mut pending = Box::pin(scheduler.sleep(Duration::from_secs(3600), &never));
            if workload == "task-sleep-pending" {
                assert!(block_on(poll_once(&mut pending)).is_none());
            }
            let tasks: Vec<_> = (0..workers * 4).map(|_| {
                let s = std::sync::Arc::clone(&scheduler);
                scheduler.spawn(async move {
                    let mut late = Vec::new();
                    for _ in 0..iterations {
                        let deadline = s.now() + Duration::from_millis(1);
                        s.sleep_until(deadline, &Cancellation::never()).await.unwrap();
                        late.push(Instant::now().duration_since(deadline).as_nanos() as f64);
                    }
                    late
                }).unwrap()
            }).collect();
            for t in tasks { lateness.extend(block_on(t).unwrap()); }
            let elapsed = start.elapsed();
            report(workload, workers, iterations, elapsed, cpu() - c0, &mut lateness);
            drop(pending);
            block_on(scheduler.shutdown());
            return;
        }
        // No load: process CPU over `iterations` milliseconds while every worker is idle.
        "idle" => {
            std::thread::sleep(Duration::from_millis(iterations as u64));
        }
        // A task woken by a timer once per second, `iterations` times: about one request per second.
        "pulse" => {
            let s = std::sync::Arc::clone(&scheduler);
            block_on(scheduler.spawn(async move {
                for _ in 0..iterations {
                    s.sleep(Duration::from_secs(1), &Cancellation::never()).await.unwrap();
                }
            }).unwrap()).unwrap();
        }
        // Worst case: a worker blocks inside one poll for 200 ms while root sleeps of 10 ms are measured. Registrations alternate between workers, so some land on the blocked one.
        "stall" => {
            for _ in 0..iterations {
                let blocker = scheduler.spawn(async { std::thread::sleep(Duration::from_millis(200)) }).unwrap();
                std::thread::sleep(Duration::from_millis(5));
                for _ in 0..8 {
                    let deadline = Instant::now() + Duration::from_millis(10);
                    block_on(scheduler.sleep_until(deadline, &never)).unwrap();
                    lateness.push(Instant::now().duration_since(deadline).as_nanos() as f64);
                }
                block_on(blocker).unwrap();
            }
        }
        // 16 tasks per worker yield `iterations` times each, with no timers: the scheduler loop alone.
        "yield" => {
            let tasks: Vec<_> = (0..workers * 16).map(|_| scheduler.spawn(async move {
                for _ in 0..iterations { futures_lite::future::yield_now().await; }
            }).unwrap()).collect();
            for task in tasks { block_on(task).unwrap(); }
        }
        _ => panic!("workload"),
    }
    let elapsed = start.elapsed();
    report(workload, workers, iterations, elapsed, cpu() - c0, &mut lateness);
    block_on(scheduler.shutdown());
}

fn report(workload: &str, workers: usize, iterations: usize, elapsed: Duration, cpu: f64, lateness: &mut Vec<f64>) {
    lateness.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let q = |p: f64| if lateness.is_empty() { 0.0 } else { lateness[((lateness.len() - 1) as f64 * p) as usize] };
    println!("{{\"workload\":\"{workload}\",\"workers\":{workers},\"iterations\":{iterations},\"ns_per_op\":{},\"cpu_ns_per_op\":{},\"p50\":{},\"p99\":{},\"max\":{}}}",
        elapsed.as_nanos() as f64 / iterations as f64, cpu * 1e9 / iterations as f64, q(0.5), q(0.99), q(1.0));
}
