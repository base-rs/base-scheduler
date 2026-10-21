// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler_timer_benchmark::{Mode, run};
use std::{sync::Arc, time::Duration};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let backend = args.get(1).map(String::as_str).unwrap_or("native");
    let mode_name = args.get(2).map(String::as_str).unwrap_or("after");
    let mode = Mode::parse(mode_name);
    let workload = args.get(3).cloned().unwrap_or("refresh".into());
    let tasks: usize = args.get(4).map(|s| s.parse().unwrap()).unwrap_or(1);
    let iterations: usize = args.get(5).map(|s| s.parse().unwrap()).unwrap_or(100_000);
    let workers: usize = args.get(6).map(|s| s.parse().unwrap()).unwrap_or(1);
    let delay = Duration::from_micros(args.get(7).map(|s| s.parse().unwrap()).unwrap_or(1000));
    let mut result = match backend {
        "tokio" => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(workers)
                .enable_all()
                .build()
                .unwrap();
            let scheduler = Arc::new(core_scheduler_tokio::TokioScheduler::new(
                runtime.handle().clone(),
            ));
            let result = runtime.block_on(run(
                scheduler.clone(),
                mode,
                workload.clone(),
                tasks,
                iterations,
                delay,
            ));
            runtime.block_on(scheduler.shutdown());
            result
        }
        "direct" => {
            let scheduler = Arc::new(
                core_scheduler_direct::DirectScheduler::new(core_scheduler_direct::DirectOptions {
                    threads: workers,
                    capacity: tasks.max(64),
                })
                .unwrap(),
            );
            let result = futures_lite::future::block_on(run(
                scheduler.clone(),
                mode,
                workload.clone(),
                tasks,
                iterations,
                delay,
            ));
            futures_lite::future::block_on(scheduler.shutdown());
            result
        }
        "native" => {
            let scheduler = Arc::new(
                core_scheduler_native::NativeScheduler::with_options(
                    core_scheduler_native::NativeOptions {
                        workers,
                        timer_capacity: tasks.max(1024),
                        ..Default::default()
                    },
                )
                .unwrap(),
            );
            let result = scheduler.block_on(run(
                scheduler.clone(),
                mode,
                workload.clone(),
                tasks,
                iterations,
                delay,
            ));
            scheduler.block_on(scheduler.shutdown());
            result
        }
        #[cfg(target_os = "linux")]
        "iouring" => {
            let (selector, drivers) =
                core_scheduler_iouring::IoUringSelector::with_workers(Default::default(), workers)
                    .unwrap();
            let scheduler = Arc::new(
                core_scheduler_native::NativeScheduler::with_drivers(
                    selector,
                    drivers,
                    core_scheduler_native::NativeExecutorOptions {
                        workers,
                        timer_capacity: tasks.max(1024),
                    },
                )
                .unwrap(),
            );
            let result = scheduler.block_on(run(
                scheduler.clone(),
                mode,
                workload.clone(),
                tasks,
                iterations,
                delay,
            ));
            scheduler.block_on(scheduler.shutdown());
            result
        }
        _ => panic!("unknown backend"),
    };
    result["backend"] = backend.into();
    result["mode"] = mode_name.into();
    result["workload"] = workload.into();
    result["tasks"] = tasks.into();
    result["iterations"] = iterations.into();
    result["workers"] = workers.into();
    result["delay_us"] = (delay.as_micros() as u64).into();
    println!("{result}");
}
