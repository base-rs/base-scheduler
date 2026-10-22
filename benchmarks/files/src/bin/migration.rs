// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#[cfg(target_os = "linux")]
mod linux {
    use core_scheduler::{
        Cancellation, FileClose, FileMonitor, FileMonitoring, FileOpen, FileOptions,
    };
    use core_scheduler_iouring::{IoUringFileRegistration, IoUringOptions, IoUringSelector};
    use core_scheduler_selector::{NativeFile, SelectorDriver, SelectorWorker};
    use futures_lite::future::block_on;
    use std::{
        future::Future,
        pin::pin,
        sync::{Arc, mpsc},
        task::{Context, Poll, Waker},
        time::Instant,
    };

    type Monitor = FileMonitor<NativeFile, IoUringFileRegistration>;
    type Payload = (Monitor, Vec<u8>);
    struct Batch {
        payload: Payload,
        count: usize,
    }

    fn affinity() -> libc::cpu_set_t {
        // SAFETY: The syscall initializes exactly the supplied cpu_set_t storage.
        let mut mask = unsafe { std::mem::zeroed::<libc::cpu_set_t>() };
        assert_eq!(
            unsafe { libc::sched_getaffinity(0, std::mem::size_of_val(&mask), &mut mask) },
            0
        );
        mask
    }
    fn pin_thread(cpu: usize) {
        // SAFETY: Both libc helpers receive valid cpu_set_t storage and the CPU comes from the process's allowed set.
        let mut mask = unsafe { std::mem::zeroed::<libc::cpu_set_t>() };
        unsafe {
            libc::CPU_SET(cpu, &mut mask);
        }
        assert_eq!(
            unsafe { libc::sched_setaffinity(0, std::mem::size_of_val(&mask), &mask) },
            0
        );
    }
    struct RestoreAffinity(libc::cpu_set_t);
    impl Drop for RestoreAffinity {
        fn drop(&mut self) {
            // SAFETY: Restore the calling thread's previously obtained affinity mask.
            assert_eq!(
                unsafe { libc::sched_setaffinity(0, std::mem::size_of_val(&self.0), &self.0) },
                0
            );
        }
    }

    pub fn run(
        workers: usize,
        burst: usize,
        rounds: usize,
        size: usize,
        write: bool,
        registered: bool,
    ) -> serde_json::Value {
        assert!(workers > 0 && burst > 0 && rounds > 0 && size > 0);
        let mask = affinity();
        let cpus: Vec<_> = (0..libc::CPU_SETSIZE as usize)
            .filter(|cpu| unsafe { libc::CPU_ISSET(*cpu, &mask) })
            .collect();
        let restore = RestoreAffinity(mask);
        let worker_offset = usize::from(cpus.len() > 1);
        let worker_cpus: Vec<_> = (0..workers)
            .map(|index| cpus[worker_offset + index % (cpus.len() - worker_offset)])
            .collect();
        let (selector, drivers) = IoUringSelector::with_workers(
            IoUringOptions {
                fixed_files: if registered { 256 } else { 0 },
                ..Default::default()
            },
            workers,
        )
        .unwrap();
        let selector = Arc::new(selector);
        let (reply, returned) = mpsc::channel::<Payload>();
        let (started, startup) = mpsc::channel::<(usize, Waker)>();
        let mut senders = Vec::new();
        let mut threads = Vec::new();
        for (index, driver) in drivers.into_iter().enumerate() {
            let (sender, inbox) = mpsc::channel::<Batch>();
            senders.push(sender);
            let selector = Arc::clone(&selector);
            let reply = reply.clone();
            let started = started.clone();
            let cpu = worker_cpus[index];
            threads.push(std::thread::spawn(move || {
                pin_thread(cpu);
                let mut worker = driver.enter();
                let waker = worker.waker();
                started.send((index, waker.clone())).unwrap();
                let mut context = Context::from_waker(&waker);
                loop {
                    if let Ok(Batch {
                        payload: (mut monitor, mut buffer),
                        count,
                    }) = inbox.try_recv()
                    {
                        let never = Cancellation::never();
                        let mut operation = pin!(async {
                            for _ in 0..count {
                                let result;
                                (result, buffer) = if write {
                                    selector
                                        .file_monitor_write_at(&mut monitor, buffer, 0, &never)
                                        .await
                                } else {
                                    selector
                                        .file_monitor_read_at(&mut monitor, buffer, 0, &never)
                                        .await
                                };
                                assert_eq!(result.unwrap(), size);
                                assert_eq!(buffer[0], 42);
                                std::hint::black_box(&buffer);
                            }
                            (monitor, buffer)
                        });
                        let payload = loop {
                            if let Poll::Ready(payload) = operation.as_mut().poll(&mut context) {
                                break payload;
                            }
                            worker.turn(None).unwrap();
                        };
                        reply.send(payload).unwrap();
                    }
                    if worker.turn(None).unwrap() {
                        break;
                    }
                }
            }));
        }
        drop((reply, started));
        let mut wakers = vec![None; workers];
        for _ in 0..workers {
            let (index, waker) = startup.recv().unwrap();
            wakers[index] = Some(waker);
        }
        let wakers: Vec<_> = wakers.into_iter().map(Option::unwrap).collect();
        pin_thread(cpus[0]);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("data");
        std::fs::write(&path, vec![42; size]).unwrap();
        let never = Cancellation::never();
        let file = block_on(selector.file_open(
            &path,
            FileOptions {
                read: true,
                write: true,
                ..Default::default()
            },
            &never,
        ))
        .unwrap();
        let mut payload = Some((selector.file_monitor(&file).unwrap(), vec![42; size]));
        let mut exchange = |index: usize, count: usize| {
            senders[index]
                .send(Batch {
                    payload: payload.take().unwrap(),
                    count,
                })
                .unwrap();
            wakers[index].wake_by_ref();
            payload = Some(returned.recv().unwrap());
        };
        // Visit every ring before timing. Registration, request storage, and driver queues are warm.
        for index in 0..workers {
            exchange(index, burst);
        }
        // Start timing immediately after the last warm route; the first measured batch remains on that ring.
        let mut times = Vec::with_capacity(rounds);
        let start = Instant::now();
        for round in 0..rounds {
            let batch_start = Instant::now();
            exchange((workers - 1 + round) % workers, burst);
            times.push(batch_start.elapsed().as_nanos() as u64);
        }
        let loop_elapsed = start.elapsed();
        let (mut monitor, buffer) = payload.take().unwrap();
        block_on(selector.file_monitor_close(&mut monitor, &never)).unwrap();
        let elapsed = start.elapsed();
        drop((monitor, buffer));
        block_on(selector.file_close(file)).unwrap();
        block_on(selector.shutdown());
        for thread in threads {
            thread.join().unwrap();
        }
        times.sort_unstable();
        drop(restore);
        let migrations = if workers == 1 { 0 } else { rounds - 1 };
        serde_json::json!({
            "coordinator_cpu": cpus[0], "worker_cpus": worker_cpus,
            "workers": workers, "burst": burst, "rounds": rounds, "size": size,
            "write": write, "registered": registered, "operations": rounds * burst,
            "migrations": migrations, "seconds": elapsed.as_secs_f64(),
            "loop_seconds": loop_elapsed.as_secs_f64(),
            "operations_per_second": (rounds * burst) as f64 / elapsed.as_secs_f64(),
            "mean_batch_ns": loop_elapsed.as_nanos() as f64 / rounds as f64,
            "p50_batch_ns": times[times.len()/2],
            "p99_batch_ns": times[(times.len()*99/100).min(times.len()-1)],
        })
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    {
        let args: Vec<String> = std::env::args().collect();
        let number = |index: usize, default: usize| {
            args.get(index)
                .map(|value| value.parse().unwrap())
                .unwrap_or(default)
        };
        let result = linux::run(
            number(1, 2),
            number(2, 64),
            number(3, 1000),
            number(4, 4096),
            args.get(5).is_some_and(|value| value == "write"),
            !args.get(6).is_some_and(|value| value == "unregistered"),
        );
        println!("{result}");
    }
    #[cfg(not(target_os = "linux"))]
    println!("This experiment requires Linux.");
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[test]
    fn owned_state_moves_between_entered_workers_and_closes() {
        for workers in [1, 2] {
            for registered in [false, true] {
                for write in [false, true] {
                    let sample = super::linux::run(workers, 2, 4, 4096, write, registered);
                    assert_eq!(sample["operations"], 8);
                    assert_eq!(sample["migrations"], if workers == 1 { 0 } else { 3 });
                }
            }
        }
    }
}
