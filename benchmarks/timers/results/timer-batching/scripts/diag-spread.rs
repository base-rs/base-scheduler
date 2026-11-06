//! Diagnostic periodic-timer workload. Both server variants compile this exact file.
use base_scheduler::{Cancellation, Clock, Spawn};
use hdrhistogram::Histogram;
use std::{sync::Arc, time::{Duration, Instant}};

struct Window { start: Instant, end: Instant, delay: Duration, aligned: bool, spread: bool, count: u32, work: u64 }
// Spread: task `index` of `count` has a fixed phase of index/count periods, so its deadlines do not depend on when it ran.
fn target(window: &Window, index: u32) -> Instant {
    let now = Instant::now();
    if !window.aligned && !window.spread { return now + window.delay; }
    let mut epoch = window.start - Duration::from_secs(1);
    if window.spread { epoch += window.delay * index / window.count; }
    let ticks = now.duration_since(epoch).as_nanos() / window.delay.as_nanos() + 1;
    epoch + window.delay * u32::try_from(ticks).unwrap()
}
fn work(window: &Window) {
    let mut value = 1u64;
    for index in 0..window.work { value = std::hint::black_box(value.wrapping_mul(6364136223846793005).wrapping_add(index)); }
    std::hint::black_box(value);
}
fn histogram() -> Histogram<u64> { Histogram::new_with_bounds(1, 60_000_000_000, 3).unwrap() }
fn record(histogram: &mut Histogram<u64>, target: Instant, now: Instant, window: &Window) {
    if target >= window.start && now < window.end {
        histogram.record(now.saturating_duration_since(target).as_nanos().max(1) as u64).unwrap();
    }
}
async fn native_task<S: Clock>(scheduler: &S, window: &Window, index: u32) -> Histogram<u64> {
    let never = Cancellation::never();
    let mut samples = histogram();
    while Instant::now() < window.end {
        let target = target(&window, index);
        scheduler.sleep_until(target, &never).await.unwrap();
        let now = Instant::now();
        assert!(now >= target, "timer fired early");
        record(&mut samples, target, now, window);
        work(window);
    }
    samples
}
async fn native_run<S: Clock + Spawn + Send + Sync + 'static>(scheduler: Arc<S>, count: usize, window: Arc<Window>) -> Histogram<u64> {
    let mut tasks = Vec::with_capacity(count);
    for index in 0..count as u32 {
        let shared = Arc::clone(&scheduler);
        let window = Arc::clone(&window);
        tasks.push(scheduler.spawn(async move { native_task(shared.as_ref(), &window, index).await }).unwrap());
    }
    let mut combined = histogram();
    for task in tasks { combined.add(task.await.unwrap()).unwrap(); }
    combined
}
fn window(delay: Duration, count: usize) -> Arc<Window> {
    let start = Instant::now() + Duration::from_secs(1);
    Arc::new(Window { start, end: start + Duration::from_secs(3), delay, aligned: std::env::var_os("CORE_TIMER_ALIGNED").is_some(), spread: std::env::var_os("CORE_TIMER_SPREAD").is_some(), count: count as u32, work: std::env::var("CORE_TIMER_WORK").unwrap_or_else(|_| "0".into()).parse().unwrap() })
}
fn cpu() -> f64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    let t = |v: libc::timeval| v.tv_sec as f64 + v.tv_usec as f64 / 1e6;
    t(usage.ru_utime) + t(usage.ru_stime)
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 5, "timers BACKEND WORKERS TASKS DELAY_US");
    let workers: usize = args[2].parse().unwrap();
    let count: usize = args[3].parse().unwrap();
    let delay = Duration::from_micros(args[4].parse().unwrap());
    let cpu_start = cpu();
    let samples = match args[1].as_str() {
        "iouring" => {
            use base_scheduler_iouring::{IoUringSelector, IoUringOptions};
            use base_scheduler_native::{NativeScheduler, NativeExecutorOptions};
            let (selector, drivers) = IoUringSelector::with_workers(IoUringOptions::default(), workers).unwrap();
            let scheduler = Arc::new(NativeScheduler::with_drivers(selector, drivers, NativeExecutorOptions { workers, timer_capacity: count * 2 + 256, ..Default::default() }).unwrap());
            let samples = scheduler.block_on(native_run(Arc::clone(&scheduler), count, window(delay, count)));
            scheduler.block_on(scheduler.shutdown());
            samples
        }
        "native" => {
            use base_scheduler_native::{NativeScheduler, NativeOptions};
            let scheduler = Arc::new(NativeScheduler::with_options(NativeOptions { workers, timer_capacity: count * 2 + 256, ..Default::default() }).unwrap());
            let samples = scheduler.block_on(native_run(Arc::clone(&scheduler), count, window(delay, count)));
            scheduler.block_on(scheduler.shutdown());
            samples
        }
        _ => panic!("unknown backend"),
    };
    println!("{}", serde_json::json!({"backend":args[1], "workers":workers, "tasks":count, "delay_us":delay.as_micros(), "seconds":3, "completed":samples.len(), "expiries_per_second":samples.len() as f64/3.0, "lateness_p50_us":samples.value_at_quantile(0.5) as f64/1000.0, "lateness_p99_us":samples.value_at_quantile(0.99) as f64/1000.0, "lateness_max_us":samples.max() as f64/1000.0, "cpu_seconds": cpu() - cpu_start}));
}
