// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use base_scheduler::{
    Cancellation, Clock, SocketClose, SocketDomain, SocketStreamReader, SocketStreamReading,
    SocketStreamWriter, SocketStreamWriting, Spawn,
};
use base_scheduler_native::{NativeOptions, NativeScheduler};
use std::{sync::Arc, time::Duration};

fn scheduler(workers: usize) -> Arc<NativeScheduler> {
    Arc::new(
        NativeScheduler::with_options(NativeOptions {
            workers,
            ..Default::default()
        })
        .unwrap(),
    )
}

#[test]
fn default_workers_drive_full_duplex_streams_and_external_root_io() {
    for workers in [1, 3] {
        let scheduler = scheduler(workers);
        scheduler.block_on(async {
            scheduler
                .timeout(Duration::from_secs(10), async {
                    let (client, server) = base_scheduler_conformance::socket::stream_pair(
                        &*scheduler,
                        SocketDomain::Ipv4,
                    )
                    .await;
                    let shared = Arc::clone(&scheduler);
                    let task = scheduler
                        .spawn(async move {
                            {
                                let mut reader = shared.socket_stream_reader(&server).unwrap();
                                let mut writer = shared.socket_stream_writer(&server).unwrap();
                                for expected in 0..128u8 {
                                    let mut byte = [0];
                                    assert_eq!(
                                        reader
                                            .read_into(&mut byte, &Cancellation::never())
                                            .await
                                            .unwrap(),
                                        1
                                    );
                                    assert_eq!(byte, [expected]);
                                    assert_eq!(
                                        writer.write(&byte, &Cancellation::never()).await.unwrap(),
                                        1
                                    );
                                }
                                reader.close().await.unwrap();
                                writer.close().await.unwrap();
                            }
                            shared.socket_close(server).await.unwrap();
                        })
                        .unwrap();
                    {
                        let mut reader = scheduler.socket_stream_reader(&client).unwrap();
                        let mut writer = scheduler.socket_stream_writer(&client).unwrap();
                        for value in 0..128u8 {
                            assert_eq!(
                                writer
                                    .write(&[value], &Cancellation::never())
                                    .await
                                    .unwrap(),
                                1
                            );
                            let mut byte = [0];
                            assert_eq!(
                                reader
                                    .read_into(&mut byte, &Cancellation::never())
                                    .await
                                    .unwrap(),
                                1
                            );
                            assert_eq!(byte, [value]);
                        }
                        reader.close().await.unwrap();
                        writer.close().await.unwrap();
                    }
                    scheduler.socket_close(client).await.unwrap();
                    task.await.unwrap();
                })
                .await
                .unwrap();
            scheduler.shutdown().await;
        });
    }
}

#[test]
fn worker_of_another_scheduler_preserves_explicit_selection() {
    let selected = scheduler(2);
    let executing = scheduler(1);
    let (client, server) = selected.block_on(base_scheduler_conformance::socket::stream_pair(
        &*selected,
        SocketDomain::Ipv4,
    ));
    let (started, waiting) = std::sync::mpsc::channel();
    let shared = Arc::clone(&selected);
    let task = executing
        .spawn(async move {
            {
                let mut reader = shared.socket_stream_reader(&server).unwrap();
                let mut byte = [0];
                let cancellation = Cancellation::never();
                let mut reading = Box::pin(reader.read_into(&mut byte, &cancellation));
                assert!(
                    futures_lite::future::poll_once(reading.as_mut())
                        .await
                        .is_none()
                );
                started.send(()).unwrap();
                assert_eq!(reading.await.unwrap(), 1);
                assert_eq!(byte, [42]);
                reader.close().await.unwrap();
            }
            shared.socket_close(server).await.unwrap();
        })
        .unwrap();
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    // The task executor may retire its own driver while live tasks keep using
    // the explicitly selected scheduler. An implicit TLS route would fail here.
    executing.close();
    selected.block_on(async {
        {
            let mut writer = selected.socket_stream_writer(&client).unwrap();
            writer.write(&[42], &Cancellation::never()).await.unwrap();
            writer.close().await.unwrap();
        }
        selected
            .timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        selected.socket_close(client).await.unwrap();
        selected.shutdown().await;
        executing.shutdown().await;
    });
}

#[test]
fn shutdown_wakes_a_worker_waiting_for_input() {
    let scheduler = scheduler(2);
    let (client, server) = scheduler.block_on(base_scheduler_conformance::socket::stream_pair(
        &*scheduler,
        SocketDomain::Ipv4,
    ));
    let shared = Arc::clone(&scheduler);
    let (started, waiting) = std::sync::mpsc::channel();
    let task = scheduler
        .spawn(async move {
            let mut reader = shared.socket_stream_reader(&server).unwrap();
            let mut byte = [0];
            let cancellation = Cancellation::never();
            let mut read = std::pin::pin!(reader.read_into(&mut byte, &cancellation));
            assert!(
                futures_lite::future::poll_once(read.as_mut())
                    .await
                    .is_none()
            );
            started.send(()).unwrap();
            read.await.unwrap_err().kind()
        })
        .unwrap();
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    scheduler.close();
    assert_eq!(
        scheduler.block_on(task).unwrap(),
        std::io::ErrorKind::BrokenPipe
    );
    drop(client);
    scheduler.block_on(scheduler.shutdown());
}

// Windows selector reads attempt the syscall on every poll, so only Unix readers wait on cached readiness.
#[cfg(unix)]
#[test]
fn immediate_reads_observe_input_while_the_only_worker_is_busy() {
    use futures_lite::future::{block_on, poll_once};
    let scheduler = scheduler(1);
    let never = Cancellation::never();
    block_on(async {
        let (client, server) =
            base_scheduler_conformance::socket::stream_pair(&*scheduler, SocketDomain::Ipv4).await;
        let mut reader = scheduler.socket_stream_reader(&server).unwrap();
        let mut writer = scheduler.socket_stream_writer(&client).unwrap();
        let mut data = [0; 4];
        assert_eq!(writer.write(b"a", &never).await.unwrap(), 1);
        assert_eq!(reader.read_into(&mut data, &never).await.unwrap(), 1);
        let (started, busy) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let task = scheduler
            .spawn(async move {
                started.send(()).unwrap();
                released.recv().unwrap();
            })
            .unwrap();
        busy.recv_timeout(Duration::from_secs(10)).unwrap();
        // With the worker busy, its driver publishes no readiness, so a readiness-first read stays pending once cached readiness is clear.
        assert!(
            poll_once(reader.read_into(&mut data, &never))
                .await
                .is_none()
        );
        assert_eq!(writer.write(b"b", &never).await.unwrap(), 1);
        assert!(
            poll_once(reader.read_into(&mut data, &never))
                .await
                .is_none()
        );
        let start = std::time::Instant::now();
        let count = loop {
            match reader.try_read_into(&mut data) {
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(start.elapsed() < Duration::from_secs(10));
                    std::thread::sleep(Duration::from_millis(1));
                }
                result => break result.unwrap(),
            }
        };
        assert_eq!((count, data[0]), (1, b'b'));
        release.send(()).unwrap();
        task.await.unwrap();
        reader.close().await.unwrap();
        writer.close().await.unwrap();
        drop((reader, writer));
        scheduler.socket_close(client).await.unwrap();
        scheduler.socket_close(server).await.unwrap();
    });
}
