// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::BufferPool;
use std::{
    hint::black_box,
    sync::{Arc, Barrier},
    time::Instant,
};

fn measure(threads: usize, iterations: usize, shared: bool, frozen: bool) -> serde_json::Value {
    let pool = BufferPool::new(threads.max(8), 4096).unwrap();
    let barrier = Arc::new(Barrier::new(threads + 1));
    let mut workers = Vec::new();
    for _ in 0..threads {
        let pool = if shared {
            pool.clone()
        } else {
            BufferPool::new(8, 4096).unwrap()
        };
        let barrier = Arc::clone(&barrier);
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            for _ in 0..iterations {
                let mut buffer = pool.try_acquire().unwrap();
                black_box(buffer.as_mut())[0] = 42;
                if frozen {
                    let buffer = buffer.freeze();
                    black_box(buffer.view(..64));
                } else {
                    black_box(buffer);
                }
            }
        }));
    }
    let started = Instant::now();
    barrier.wait();
    for worker in workers {
        worker.join().unwrap();
    }
    let seconds = started.elapsed().as_secs_f64();
    serde_json::json!({"threads":threads,"iterations":iterations,"shared":shared,"frozen":frozen,"seconds":seconds,"leases_per_second":(threads * iterations) as f64 / seconds})
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    println!(
        "{}",
        measure(
            args.get(1).map(|value| value.parse().unwrap()).unwrap_or(1),
            args.get(2)
                .map(|value| value.parse().unwrap())
                .unwrap_or(1_000_000),
            args.get(3).is_none_or(|value| value == "shared"),
            args.get(4).is_some_and(|value| value == "frozen"),
        )
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_and_local_leases_can_be_moved_and_frozen() {
        for shared in [false, true] {
            for frozen in [false, true] {
                let result = measure(2, 100, shared, frozen);
                assert!(result["leases_per_second"].as_f64().unwrap() > 0.0);
            }
        }
    }
}
