// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::*;
use std::{future::Future, sync::Arc, time::Instant};

async fn read_pooled<R: FileReader, P: BufferPool>(
    reader: &mut R,
    pool: &P,
    cancellation: &Cancellation,
) -> OperationResult<Option<P::Buffer>> {
    let buffer = pool.acquire(cancellation).await?;
    let (result, mut buffer) = reader.read(buffer, cancellation).await;
    let count = result?;
    if count == 0 {
        return Ok(None);
    }
    buffer.truncate(count);
    Ok(Some(buffer))
}

// A submitted file transfer can complete short. Retry only the untransferred suffix, retaining its original owner until all access ends.
async fn finish_write<W: FileWriter, B: IoBuf>(
    writer: &mut W,
    completion: impl Future<Output = BufferResult<B>>,
    cancellation: &Cancellation,
) -> OperationResult<()> {
    let (result, buffer) = completion.await;
    let written = result?;
    let length = buffer.as_ref().len();
    if written < length {
        if written == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::WriteZero).into());
        }
        let (result, _) = writer
            .write_all(buffer.slice(written..), cancellation)
            .await;
        assert_eq!(result?, length - written);
    }
    Ok(())
}

async fn run<S>(scheduler: &S, mode: &str, size: usize, bytes: usize) -> serde_json::Value
where
    S: FileReading + FileWriting + FileOpen + FileClose + BufferPooling,
    <S::Pool as BufferPool>::Buffer: std::fmt::Debug,
{
    assert!(size > 0, "buffer size must be positive");
    let directory = tempfile::tempdir().unwrap();
    let input_path = directory.path().join("input");
    let output_path = directory.path().join("output");
    let expected: Vec<u8> = (0..bytes)
        .map(|i| (i.wrapping_mul(31) ^ (i >> 12)) as u8)
        .collect();
    std::fs::write(&input_path, &expected).unwrap();
    // Prepopulate output pages too: isolate dispatch, overlap, and memory movement from file allocation and durability.
    std::fs::write(&output_path, vec![0; bytes]).unwrap();
    let never = Cancellation::never();
    let mut seconds = 0.0;
    for pass in 0..2 {
        let input = scheduler
            .file_open(&input_path, FileOptions::READ, &never)
            .await
            .unwrap();
        let output = scheduler
            .file_open(
                &output_path,
                FileOptions {
                    write: true,
                    ..Default::default()
                },
                &never,
            )
            .await
            .unwrap();
        let pool = scheduler
            .buffer_pool(BufferPoolOptions::new(2, size))
            .unwrap();
        let mut reader = scheduler.file_reader(&input).unwrap();
        let mut writer = scheduler.file_writer(&output).unwrap();
        let started = Instant::now();
        let mut next = if mode == "vec-join" || mode == "vec-serial" {
            let mut buffer = vec![0; size];
            let mut spare = vec![0; size];
            let count;
            (count, buffer) = reader.read(buffer, &never).await;
            let mut count = count.unwrap();
            let mut copied = 0;
            while count != 0 {
                let (written, read) = if mode == "vec-join" {
                    futures_lite::future::zip(
                        writer.write_all(buffer.slice(..count), &never),
                        reader.read(spare, &never),
                    )
                    .await
                } else {
                    let written = writer.write_all(buffer.slice(..count), &never).await;
                    let read = reader.read(spare, &never).await;
                    (written, read)
                };
                assert_eq!(written.0.unwrap(), count);
                copied += count;
                spare = written.1.into_inner();
                buffer = read.1;
                count = read.0.unwrap();
            }
            assert_eq!(copied, bytes);
            None
        } else {
            read_pooled(&mut reader, &pool, &never).await.unwrap()
        };
        let mut copied = 0;
        while let Some(buffer) = next {
            let length = buffer.as_ref().len();
            next = match mode {
                "serial" => {
                    let (result, buffer) = writer.write_all(buffer, &never).await;
                    assert_eq!(result.unwrap(), length);
                    drop(buffer);
                    read_pooled(&mut reader, &pool, &never).await.unwrap()
                }
                "join" => {
                    let (written, received) = futures_lite::future::zip(
                        async {
                            let (result, buffer) = writer.write_all(buffer, &never).await;
                            drop(buffer);
                            result
                        },
                        read_pooled(&mut reader, &pool, &never),
                    )
                    .await;
                    assert_eq!(written.unwrap(), length);
                    received.unwrap()
                }
                "submit" => {
                    let completion = writer.write_submit(buffer, &never).await.unwrap();
                    // The accepted write progresses while only the input completion is being polled.
                    let received = read_pooled(&mut reader, &pool, &never).await;
                    finish_write(&mut writer, completion, &never).await.unwrap();
                    received.unwrap()
                }
                _ => unreachable!("The pipeline mode was validated before execution"),
            };
            copied += length;
        }
        let elapsed = started.elapsed();
        if mode != "vec-join" && mode != "vec-serial" {
            assert_eq!(copied, bytes);
        }
        reader.close().await.unwrap();
        writer.close().await.unwrap();
        drop((reader, writer));
        scheduler.file_close(input).await.unwrap();
        scheduler.file_close(output).await.unwrap();
        assert_eq!(std::fs::read(&output_path).unwrap(), expected);
        if pass == 1 {
            seconds = elapsed.as_secs_f64();
        }
    }
    serde_json::json!({"mode":mode,"size":size,"bytes":bytes,"seconds":seconds,"gib_per_second":bytes as f64 / seconds / (1024.0*1024.0*1024.0)})
}

fn execute(args: &[String]) -> serde_json::Value {
    let backend = args.get(1).map(String::as_str).unwrap_or("native");
    let mode = args.get(2).map(String::as_str).unwrap_or("join");
    assert!(
        matches!(
            mode,
            "vec-serial" | "vec-join" | "serial" | "join" | "submit"
        ),
        "unknown pipeline mode"
    );
    let size: usize = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(65536);
    let bytes: usize = args
        .get(4)
        .map(|s| s.parse().unwrap())
        .unwrap_or(64 * 1024 * 1024);
    let workers: usize = args.get(5).map(|s| s.parse().unwrap()).unwrap_or(1);
    let mut result = match backend {
        "direct" => {
            let scheduler =
                core_scheduler_direct::DirectScheduler::new(core_scheduler_direct::DirectOptions {
                    threads: workers,
                    capacity: 64,
                })
                .unwrap();
            let result = futures_lite::future::block_on(run(&scheduler, mode, size, bytes));
            futures_lite::future::block_on(scheduler.shutdown());
            result
        }
        "tokio" => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(workers)
                .enable_all()
                .build()
                .unwrap();
            let scheduler = Arc::new(core_scheduler_tokio::TokioScheduler::new(
                runtime.handle().clone(),
            ));
            let owned = scheduler.clone();
            let mode = mode.to_owned();
            let result = runtime
                .block_on(
                    runtime.spawn(async move { run(owned.as_ref(), &mode, size, bytes).await }),
                )
                .unwrap();
            runtime.block_on(scheduler.shutdown());
            result
        }
        "native" => {
            let scheduler = Arc::new(
                core_scheduler_native::NativeScheduler::with_options(
                    core_scheduler_native::NativeOptions {
                        workers,
                        ..Default::default()
                    },
                )
                .unwrap(),
            );
            let owned = Arc::clone(&scheduler);
            let mode = mode.to_owned();
            let result = scheduler
                .block_on(
                    scheduler
                        .spawn(async move { run(owned.as_ref(), &mode, size, bytes).await })
                        .unwrap(),
                )
                .unwrap();
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
                        ..Default::default()
                    },
                )
                .unwrap(),
            );
            let owned = Arc::clone(&scheduler);
            let mode = mode.to_owned();
            let result = scheduler
                .block_on(
                    scheduler
                        .spawn(async move { run(owned.as_ref(), &mode, size, bytes).await })
                        .unwrap(),
                )
                .unwrap();
            scheduler.block_on(scheduler.shutdown());
            result
        }
        _ => panic!("unknown backend"),
    };
    result["backend"] = backend.into();
    result["workers"] = workers.into();
    result
}

fn main() {
    println!("{}", execute(&std::env::args().collect::<Vec<_>>()));
}

#[cfg(test)]
#[path = "files/tests.rs"]
mod tests;
