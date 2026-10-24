// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::{
    Cancellation, FileClose, FileOpen, FileOptions, backend::FileMonitoring, backend::FileReadAt,
    backend::FileWriteAt,
};
use core_scheduler_iouring::{IoUringOptions, IoUringSelector};
use futures_lite::future::block_on;
use std::time::Instant;

// An external caller deliberately selects rings round-robin. This isolates repeated ring changes without relying on the executor's nondeterministic task-stealing policy.
pub fn run_migration(
    registered: bool,
    monitored: bool,
    write: bool,
    iterations: usize,
    workers: usize,
    size: usize,
) -> serde_json::Value {
    let (selector, drivers) = IoUringSelector::with_workers(
        IoUringOptions {
            fixed_files: if registered { 256 } else { 0 },
            ..Default::default()
        },
        workers,
    )
    .unwrap();
    let threads = drivers
        .into_iter()
        .map(|driver| std::thread::spawn(move || driver.run().unwrap()))
        .collect::<Vec<_>>();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("data");
    std::fs::write(&path, vec![42; size]).unwrap();
    let never = Cancellation::never();
    let (elapsed, task_elapsed) = block_on(async {
        let file = selector
            .file_open(
                &path,
                FileOptions {
                    read: true,
                    write: true,
                    ..Default::default()
                },
                &never,
            )
            .await
            .unwrap();
        let mut monitor = selector.file_monitor(&file).unwrap();
        let mut buffer = vec![42; size];
        // Warm every route before timing; one route is visited on every successive operation.
        for _ in 0..workers {
            let (result, returned) = selector
                .file_monitor_read_at(&mut monitor, buffer, 0, &never)
                .await;
            assert_eq!(result.unwrap(), size);
            buffer = returned;
        }
        let start = Instant::now();
        for _ in 0..iterations {
            let (result, returned) = match (monitored, write) {
                (true, false) => {
                    selector
                        .file_monitor_read_at(&mut monitor, buffer, 0, &never)
                        .await
                }
                (true, true) => {
                    selector
                        .file_monitor_write_at(&mut monitor, buffer, 0, &never)
                        .await
                }
                (false, false) => selector.file_read_at(&file, buffer, 0, &never).await,
                (false, true) => selector.file_write_at(&file, buffer, 0, &never).await,
            };
            assert_eq!(result.unwrap(), size);
            buffer = returned;
            std::hint::black_box(buffer[0]);
        }
        let task_elapsed = start.elapsed();
        selector.file_monitor_close(&mut monitor).await.unwrap();
        let elapsed = start.elapsed();
        drop(monitor);
        selector.file_close(file).await.unwrap();
        selector.shutdown().await;
        (elapsed, task_elapsed)
    });
    for thread in threads {
        thread.join().unwrap();
    }
    serde_json::json!({"operations": iterations, "seconds": elapsed.as_secs_f64(), "operations_per_second": iterations as f64 / elapsed.as_secs_f64(), "mean_operation_ns": task_elapsed.as_nanos() as f64 / iterations as f64})
}
