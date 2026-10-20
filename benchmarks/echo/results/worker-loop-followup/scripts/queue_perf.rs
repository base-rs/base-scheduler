use std::{hint::black_box, time::{Duration, Instant}};
macro_rules! measure {
    ($queue:expr, $release:expr, $workload:expr) => {{
        let mut queue = $queue;
        let mut keys = Vec::with_capacity(1024);
        let now = Instant::now();
        let start = Instant::now();
        match $workload {
            "deadline" => {
                for index in 0..1024 {
                    keys.push(queue.try_schedule(now + Duration::from_nanos(index), index).unwrap());
                }
                black_box(queue.next_deadline());
                for _ in 0..4_000_000 {
                    black_box(black_box(&mut queue).next_deadline());
                }
            }
            "expiry" => {
                for _ in 0..2000 {
                    for index in 0..1024 {
                        keys.push(queue.try_schedule(now + Duration::from_nanos(index), index).unwrap());
                    }
                    while let Some(value) = black_box(&mut queue).pop_expired(now + Duration::from_secs(1)) {
                        black_box(value);
                    }
                    for key in keys.drain(..) { ($release)(&mut queue, key); }
                }
            }
            _ => panic!("unknown workload"),
        }
        start.elapsed().as_nanos()
    }};
}
fn main() {
    let args:Vec<_> = std::env::args().collect();
    let elapsed = match args[1].as_str() {
        "shared" => measure!(core_scheduler_timers::Timers::with_capacity(1024), |_, key| drop(key), args[2].as_str()),
        #[cfg(feature="local")]
        "local" => measure!(core_scheduler_timers::local::TimerQueue::with_capacity(1024), |queue: &mut core_scheduler_timers::local::TimerQueue<u64>, key| {black_box(queue.release(key));}, args[2].as_str()),
        _ => panic!("unknown queue"),
    };
    println!("{{\"queue\":\"{}\",\"workload\":\"{}\",\"elapsed_ns\":{}}}", args[1], args[2], elapsed);
}
