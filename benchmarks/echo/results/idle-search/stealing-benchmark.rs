// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Diagnostic only: concentrate already-suspended tasks on one worker, then measure a ready burst while that worker performs a long poll.

use crate::task_service::TaskService;
use futures_channel::oneshot;
use futures_lite::future::block_on;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}, mpsc};
use std::time::{Duration, Instant};

fn parameter(name: &str, default: usize) -> usize {
    std::env::var(name).map_or(default, |value| value.parse().unwrap())
}

fn consume(duration: Duration) {
    let started = Instant::now();
    while started.elapsed() < duration {
        std::hint::spin_loop();
    }
}

#[test]
#[ignore = "controlled work-stealing performance experiment"]
fn uneven_burst() {
    let workers = parameter("STEAL_WORKERS", 4);
    let tasks = parameter("STEAL_TASKS", 128);
    let work_us = parameter("STEAL_WORK_US", 100);
    let busy_us = parameter("STEAL_BUSY_US", 20_000);
    let rounds = parameter("STEAL_ROUNDS", 10);
    assert!(workers > 0 && tasks > 0 && rounds > 0);
    let service = TaskService::new(workers, 64).unwrap();
    for round in 0..rounds {
        let mut senders = Vec::with_capacity(tasks);
        let mut handles = Vec::with_capacity(tasks);
        for _ in 0..tasks {
            let (sender, receiver) = oneshot::channel::<Instant>();
            senders.push(sender);
            handles.push(service.spawn(async move {
                let submitted = receiver.await.unwrap();
                let start_ns = submitted.elapsed().as_nanos() as u64;
                consume(Duration::from_micros(work_us as u64));
                let worker = super::CURRENT_WORKER.with(|context| context.borrow().as_ref().unwrap().identifier);
                (start_ns, submitted.elapsed().as_nanos() as u64, worker)
            }).unwrap());
        }
        // Every receiver has reached Pending, with no queued runnable or active poll that could race the diagnostic placement change.
        block_on(service.wait(false));
        let (started, location) = mpsc::channel();
        let release = Arc::new(AtomicBool::new(false));
        let held = Arc::clone(&release);
        let busy = service.spawn(async move {
            let worker = super::CURRENT_WORKER.with(|context| context.borrow().as_ref().unwrap().identifier);
            started.send(worker).unwrap();
            while !held.load(Ordering::Acquire) {
                std::hint::spin_loop();
            }
            consume(Duration::from_micros(busy_us as u64));
        }).unwrap();
        let owner = location.recv_timeout(Duration::from_secs(10)).unwrap();
        for handle in &handles {
            handle.0.metadata().worker.store(owner, Ordering::Relaxed);
        }
        let submitted = Instant::now();
        for sender in senders {
            sender.send(submitted).unwrap();
        }
        // The long poll runs for busy_us after publication, independently of how quickly other workers complete the burst.
        release.store(true, Ordering::Release);
        let results = handles.into_iter().map(|handle| block_on(handle).unwrap()).collect::<Vec<_>>();
        let elapsed_ns = submitted.elapsed().as_nanos();
        let mut start_ns = results.iter().map(|result| result.0).collect::<Vec<_>>();
        let mut finish_ns = results.iter().map(|result| result.1).collect::<Vec<_>>();
        start_ns.sort_unstable();
        finish_ns.sort_unstable();
        let mut per_worker = vec![0; workers];
        for result in &results {
            per_worker[result.2] += 1;
        }
        block_on(busy).unwrap();
        block_on(service.wait(false));
        println!("STEAL_RESULT {{\"workers\":{workers},\"tasks\":{tasks},\"work_us\":{work_us},\"busy_us\":{busy_us},\"round\":{round},\"owner\":{owner},\"elapsed_ns\":{elapsed_ns},\"start_ns\":{start_ns:?},\"finish_ns\":{finish_ns:?},\"per_worker\":{per_worker:?}}}");
    }
    service.close();
    block_on(service.drain());
}
