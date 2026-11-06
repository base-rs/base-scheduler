//! Diagnostic periodic-timer workload. Both server variants compile this exact file.
use base_scheduler::{Cancellation, Clock, Spawn};
use hdrhistogram::Histogram;
use std::{sync::Arc, time::{Duration, Instant}};

struct Window { start: Instant, end: Instant, delay: Duration, aligned: bool, work: u64 }
fn target(window: &Window) -> Instant {
    let now = Instant::now();
    if !window.aligned { return now + window.delay; }
    let epoch = window.start - Duration::from_secs(1);
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
#[derive(Clone)]
struct Probe { total: Histogram<u64>, collect: Histogram<u64>, queue: Histogram<u64>, idle_collect: Histogram<u64>, busy_collect: Histogram<u64>, overshoot: Histogram<u64>, idle_queue: Histogram<u64>, busy_queue: Histogram<u64>, moved: u64 }
fn probe() -> Probe { Probe { total: histogram(), collect: histogram(), queue: histogram(), idle_collect: histogram(), busy_collect: histogram(), overshoot: histogram(), idle_queue: histogram(), busy_queue: histogram(), moved: 0 } }
fn d(a: Instant, b: Instant) -> u64 { a.saturating_duration_since(b).as_nanos().max(1) as u64 }
async fn native_task<S: Clock>(scheduler: &S, window: &Window) -> Probe {
    let never = Cancellation::never();
    let mut samples = probe();
    while Instant::now() < window.end {
        let target = target(&window);
        let thread = std::thread::current().id();
        scheduler.sleep_until(target, &never).await.unwrap();
        let now = Instant::now();
        assert!(now >= target, "timer fired early");
        if target >= window.start && now < window.end {
            samples.total.record(d(now, target)).unwrap();
            if std::thread::current().id() == thread {
                let (pass, idle) = base_scheduler_native::probe::get();
                let pass = pass.unwrap();
                samples.collect.record(d(pass, target)).unwrap();
                samples.queue.record(d(now, pass)).unwrap();
                if let Some(idle) = idle {
                    samples.idle_collect.record(d(pass, target)).unwrap();
                    samples.overshoot.record(d(idle, target)).unwrap();
                    samples.idle_queue.record(d(now, pass)).unwrap();
                } else {
                    samples.busy_collect.record(d(pass, target)).unwrap();
                    samples.busy_queue.record(d(now, pass)).unwrap();
                }
            } else {
                samples.moved += 1;
            }
        }
        work(window);
    }
    samples
}
async fn native_run<S: Clock + Spawn + Send + Sync + 'static>(scheduler: Arc<S>, count: usize, window: Arc<Window>) -> Probe {
    let mut tasks = Vec::with_capacity(count);
    for _ in 0..count {
        let shared = Arc::clone(&scheduler);
        let window = Arc::clone(&window);
        tasks.push(scheduler.spawn(async move { native_task(shared.as_ref(), &window).await }).unwrap());
    }
    let mut c = probe();
    for task in tasks {
        let p = task.await.unwrap();
        c.total.add(p.total).unwrap(); c.collect.add(p.collect).unwrap(); c.queue.add(p.queue).unwrap();
        c.idle_collect.add(p.idle_collect).unwrap(); c.busy_collect.add(p.busy_collect).unwrap(); c.overshoot.add(p.overshoot).unwrap();
        c.idle_queue.add(p.idle_queue).unwrap(); c.busy_queue.add(p.busy_queue).unwrap(); c.moved += p.moved;
    }
    c
}
fn window(delay: Duration) -> Arc<Window> {
    let start = Instant::now() + Duration::from_secs(1);
    Arc::new(Window { start, end: start + Duration::from_secs(3), delay, aligned: std::env::var_os("CORE_TIMER_ALIGNED").is_some(), work: std::env::var("CORE_TIMER_WORK").unwrap_or_else(|_| "0".into()).parse().unwrap() })
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
    let probe = match args[1].as_str() {
        "iouring" => {
            use base_scheduler_iouring::{IoUringSelector, IoUringOptions};
            use base_scheduler_native::{NativeScheduler, NativeExecutorOptions};
            let (selector, drivers) = IoUringSelector::with_workers(IoUringOptions::default(), workers).unwrap();
            let scheduler = Arc::new(NativeScheduler::with_drivers(selector, drivers, NativeExecutorOptions { workers, timer_capacity: count * 2 + 256, ..Default::default() }).unwrap());
            let samples = scheduler.block_on(native_run(Arc::clone(&scheduler), count, window(delay)));
            scheduler.block_on(scheduler.shutdown());
            samples
        }
        "native" => {
            use base_scheduler_native::{NativeScheduler, NativeOptions};
            let scheduler = Arc::new(NativeScheduler::with_options(NativeOptions { workers, timer_capacity: count * 2 + 256, ..Default::default() }).unwrap());
            let samples = scheduler.block_on(native_run(Arc::clone(&scheduler), count, window(delay)));
            scheduler.block_on(scheduler.shutdown());
            samples
        }
        _ => panic!("unknown backend"),
    };
    let samples = &probe.total;
    let q = |h: &Histogram<u64>, q: f64| h.value_at_quantile(q) as f64 / 1000.0;
    let counts = base_scheduler_native::probe::counts();
    println!("{}", serde_json::json!({"collect_p50_us": q(&probe.collect, 0.5), "queue_p50_us": q(&probe.queue, 0.5), "idle_collect_p50_us": q(&probe.idle_collect, 0.5), "busy_collect_p50_us": q(&probe.busy_collect, 0.5), "overshoot_p50_us": q(&probe.overshoot, 0.5), "idle_queue_p50_us": q(&probe.idle_queue, 0.5), "busy_queue_p50_us": q(&probe.busy_queue, 0.5), "idle_share": probe.idle_collect.len() as f64 / probe.collect.len().max(1) as f64, "moved_share": probe.moved as f64 / samples.len() as f64, "collect_mean_us": probe.collect.mean() / 1000.0, "queue_mean_us": probe.queue.mean() / 1000.0, "passes": counts[0], "idle_passes": counts[1], "fired": counts[2], "sleeps": counts[3],"backend":args[1], "workers":workers, "tasks":count, "delay_us":delay.as_micros(), "seconds":3, "completed":samples.len(), "expiries_per_second":samples.len() as f64/3.0, "lateness_p50_us":samples.value_at_quantile(0.5) as f64/1000.0, "lateness_p99_us":samples.value_at_quantile(0.99) as f64/1000.0, "lateness_max_us":samples.max() as f64/1000.0, "cpu_seconds": cpu() - cpu_start}));
}
