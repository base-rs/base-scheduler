//! Diagnostic periodic-timer workload. Both server variants compile this exact file.
use core_scheduler::{Cancellation, Clock, Spawn};
use hdrhistogram::Histogram;
use std::{sync::Arc, time::{Duration, Instant}};

struct Window { start: Instant, end: Instant, delay: Duration }
fn histogram() -> Histogram<u64> { Histogram::new_with_bounds(1, 60_000_000_000, 3).unwrap() }
fn record(histogram: &mut Histogram<u64>, target: Instant, now: Instant, window: &Window) {
    if target >= window.start && now < window.end {
        histogram.record(now.saturating_duration_since(target).as_nanos().max(1) as u64).unwrap();
    }
}
async fn native_task<S: Clock>(scheduler: &S, window: &Window) -> Histogram<u64> {
    let never = Cancellation::never();
    let mut samples = histogram();
    while Instant::now() < window.end {
        let target = Instant::now() + window.delay;
        scheduler.sleep_until(target, &never).await.unwrap();
        let now = Instant::now();
        assert!(now >= target, "timer fired early");
        record(&mut samples, target, now, window);
    }
    samples
}
async fn native_run<S: Clock + Spawn + Send + Sync + 'static>(scheduler: Arc<S>, count: usize, window: Arc<Window>) -> Histogram<u64> {
    let mut tasks = Vec::with_capacity(count);
    for _ in 0..count {
        let shared = Arc::clone(&scheduler);
        let window = Arc::clone(&window);
        tasks.push(scheduler.spawn(async move { native_task(shared.as_ref(), &window).await }).unwrap());
    }
    let mut combined = histogram();
    for task in tasks { combined.add(task.await.unwrap()).unwrap(); }
    combined
}
fn window(delay: Duration) -> Arc<Window> {
    let start = Instant::now() + Duration::from_secs(1);
    Arc::new(Window { start, end: start + Duration::from_secs(3), delay })
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 5, "timers BACKEND WORKERS TASKS DELAY_US");
    let workers: usize = args[2].parse().unwrap();
    let count: usize = args[3].parse().unwrap();
    let delay = Duration::from_micros(args[4].parse().unwrap());
    let samples = match args[1].as_str() {
        "iouring" => {
            use core_scheduler_iouring::{IoUringSelector, IoUringOptions};
            use core_scheduler_native::{NativeScheduler, NativeExecutorOptions};
            let (selector, drivers) = IoUringSelector::with_workers(IoUringOptions::default(), workers).unwrap();
            let scheduler = Arc::new(NativeScheduler::with_drivers(selector, drivers, NativeExecutorOptions { workers, timer_capacity: count * 2 + 256 }).unwrap());
            let samples = scheduler.block_on(native_run(Arc::clone(&scheduler), count, window(delay)));
            scheduler.block_on(scheduler.shutdown());
            samples
        }
        "native" => {
            use core_scheduler_native::{NativeScheduler, NativeOptions};
            let scheduler = Arc::new(NativeScheduler::with_options(NativeOptions { workers, timer_capacity: count * 2 + 256, ..Default::default() }).unwrap());
            let samples = scheduler.block_on(native_run(Arc::clone(&scheduler), count, window(delay)));
            scheduler.block_on(scheduler.shutdown());
            samples
        }
        "tokio" => {
            let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(workers).enable_all().build().unwrap();
            runtime.block_on(async {
                let window = window(delay);
                let mut tasks = Vec::with_capacity(count);
                for _ in 0..count {
                    let window = Arc::clone(&window);
                    tasks.push(tokio::spawn(async move {
                        let mut samples = histogram();
                        while Instant::now() < window.end {
                            let target = Instant::now() + window.delay;
                            tokio::time::sleep_until(target.into()).await;
                            let now = Instant::now();
                            assert!(now >= target);
                            record(&mut samples, target, now, &window);
                        }
                        samples
                    }));
                }
                let mut combined = histogram();
                for task in tasks { combined.add(task.await.unwrap()).unwrap(); }
                combined
            })
        }
        _ => panic!("unknown backend"),
    };
    println!("{}", serde_json::json!({"backend":args[1], "workers":workers, "tasks":count, "delay_us":delay.as_micros(), "seconds":3, "completed":samples.len(), "expiries_per_second":samples.len() as f64/3.0, "lateness_p50_us":samples.value_at_quantile(0.5) as f64/1000.0, "lateness_p99_us":samples.value_at_quantile(0.99) as f64/1000.0, "lateness_max_us":samples.max() as f64/1000.0}));
}
