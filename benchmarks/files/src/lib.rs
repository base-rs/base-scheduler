// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::{Cancellation, FileClose, FileMonitoring, FileOpen, FileOptions, Spawn};
use std::{sync::Arc, time::Instant};

pub async fn run<S>(
    scheduler: Arc<S>,
    monitored: bool,
    workload: String,
    tasks: usize,
    iterations: usize,
    size: usize,
) -> serde_json::Value
where
    S: FileMonitoring + FileOpen + FileClose + Spawn + Send + Sync + 'static,
    S::File: 'static,
{
    assert!(tasks > 0 && iterations > 0 && size > 0);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("data");
    // A small page-cache-hot file isolates scheduler overhead from disk throughput. No fsync or durability claim.
    std::fs::write(&path, vec![42; size]).unwrap();
    let never = Cancellation::never();
    let file = Arc::new(
        scheduler
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
            .unwrap(),
    );
    let mut handles = Vec::new();
    // Warm up outside the timed interval, including worker startup and request allocation.
    let mut warm = scheduler.file_monitor(&file).unwrap();
    let (_, buffer) = scheduler
        .file_monitor_read_at(&mut warm, vec![0; size], 0, &never)
        .await;
    scheduler
        .file_monitor_close(&mut warm, &never)
        .await
        .unwrap();
    drop((warm, buffer));
    let started = Instant::now();
    for _ in 0..tasks {
        let scheduler = Arc::clone(&scheduler);
        let file = Arc::clone(&file);
        let workload = workload.clone();
        handles.push(
            Arc::clone(&scheduler)
                .spawn(async move {
                    let never = Cancellation::never();
                    let mut monitor = scheduler.file_monitor(&file).unwrap();
                    let mut buffer = vec![42; size];
                    let mut elapsed = 0;
                    // Measure each task's steady-state loop, excluding its first operation.
                    for index in 0..=iterations {
                        let result;
                        let operation = Instant::now();
                        (result, buffer) = match (monitored, workload.as_str()) {
                            (true, "read-at") => {
                                scheduler
                                    .file_monitor_read_at(&mut monitor, buffer, 0, &never)
                                    .await
                            }
                            (false, "read-at") => {
                                scheduler.file_read_at(&file, buffer, 0, &never).await
                            }
                            (true, "write-at") => {
                                scheduler
                                    .file_monitor_write_at(&mut monitor, buffer, 0, &never)
                                    .await
                            }
                            (false, "write-at") => {
                                scheduler.file_write_at(&file, buffer, 0, &never).await
                            }
                            _ => panic!("unknown workload"),
                        };
                        assert_eq!(result.unwrap(), size);
                        std::hint::black_box(buffer[0]);
                        if index > 0 {
                            elapsed += operation.elapsed().as_nanos() as u64;
                        }
                    }
                    scheduler
                        .file_monitor_close(&mut monitor, &never)
                        .await
                        .unwrap();
                    elapsed
                })
                .unwrap(),
        );
    }
    let mut task_ns = 0;
    for handle in handles {
        task_ns += handle.await.unwrap();
    }
    let elapsed = started.elapsed();
    let file = Arc::try_unwrap(file).ok().unwrap();
    scheduler.file_close(file).await.unwrap();
    serde_json::json!({"operations": tasks * iterations, "seconds": elapsed.as_secs_f64(), "operations_per_second": (tasks * iterations) as f64 / elapsed.as_secs_f64(), "mean_operation_ns": task_ns as f64 / (tasks * iterations) as f64})
}
