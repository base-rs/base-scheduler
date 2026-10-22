// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::*;
use std::{
    io::{IoSliceMut, Read, Write},
    net::{TcpListener, UdpSocket},
    sync::Arc,
    time::Instant,
};

async fn run<S>(
    scheduler: &S,
    mode: &str,
    size: usize,
    iterations: usize,
    transport: &str,
    window: usize,
    receive_only: bool,
) -> serde_json::Value
where
    S: SocketScheduler
        + SocketMonitoring
        + SocketReceiving
        + SocketRecvMsgMultishot
        + SocketSetOption<TcpNoDelay>,
    S::Socket: 'static,
    S::PooledReceiver: SocketRecvMsgReceiver<S, Buffer = BufferLease>,
{
    let udp = transport == "udp";
    assert!(size > 0 && iterations > 0 && window > 0 && (!udp || size <= 65507));
    let warmup = 1000;
    let (address, peer) = if udp {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        let address = socket.local_addr().unwrap();
        let peer = std::thread::spawn(move || {
            let mut bytes = vec![0; size];
            for _ in 0..iterations + warmup {
                if receive_only {
                    let (length, remote) = socket.recv_from(&mut bytes).unwrap();
                    assert_eq!(length, 1);
                    bytes.fill(42);
                    for _ in 0..window {
                        assert_eq!(socket.send_to(&bytes, remote).unwrap(), size);
                    }
                } else {
                    for _ in 0..window {
                        let (length, remote) = socket.recv_from(&mut bytes).unwrap();
                        assert_eq!(length, size);
                        assert_eq!(socket.send_to(&bytes, remote).unwrap(), size);
                    }
                }
            }
        });
        (address, peer)
    } else {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let peer = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_nodelay(true).unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let mut bytes = vec![0; size];
            for _ in 0..iterations + warmup {
                if receive_only {
                    socket.read_exact(&mut [0]).unwrap();
                    bytes.fill(42);
                    for _ in 0..window {
                        socket.write_all(&bytes).unwrap();
                    }
                } else {
                    for _ in 0..window {
                        socket.read_exact(&mut bytes).unwrap();
                        socket.write_all(&bytes).unwrap();
                    }
                }
            }
        });
        (address, peer)
    };
    let never = Cancellation::never();
    let socket = scheduler
        .socket_open(
            SocketDomain::Ipv4,
            if udp {
                SocketType::Datagram
            } else {
                SocketType::Stream
            },
            None,
            &never,
        )
        .await
        .unwrap();
    scheduler
        .socket_connect(&socket, address, &never)
        .await
        .unwrap();
    if !udp {
        scheduler
            .socket_set_option(&socket, TcpNoDelay, true, &never)
            .await
            .unwrap();
    }
    let pool = BufferPool::new(8, size).unwrap();
    let mut receiver = scheduler
        .socket_receiver(&socket, pool, SocketRecvFlags::default(), &never)
        .await
        .unwrap();
    let mut readable = scheduler
        .socket_monitor(&socket, Interest::Readable)
        .unwrap();
    let mut writable = scheduler
        .socket_monitor(&socket, Interest::Writable)
        .unwrap();
    let mut multishot = if matches!(mode, "multi" | "multi-retain") {
        Some(
            scheduler
                .socket_recvmsg_multishot(
                    &socket,
                    RecvMsgMultishotOptions {
                        buffers: (window + 2).max(8).try_into().unwrap(),
                        buffer_size: size,
                    },
                    SocketRecvFlags::default(),
                    &never,
                )
                .await
                .unwrap(),
        )
    } else {
        None
    };
    let payload = vec![42; size];
    let mut raw = vec![0; size];
    let mut retained = None;
    let mut retained_multi = None;
    let mut start = Instant::now();
    for index in 0..iterations + warmup {
        if index == warmup {
            start = Instant::now();
        }
        let outgoing = if receive_only {
            &payload[..1]
        } else {
            &payload[..]
        };
        for _ in 0..if receive_only { 1 } else { window } {
            let mut sent = 0;
            while sent < outgoing.len() {
                let count = scheduler
                    .socket_monitor_send_poll(
                        &mut writable,
                        &outgoing[sent..],
                        SocketSendFlags::default(),
                        &never,
                    )
                    .await
                    .unwrap();
                assert!(count > 0);
                if udp {
                    assert_eq!(count, outgoing.len());
                }
                sent += count;
            }
        }
        let mut received = 0;
        while received < size * window {
            let count = match mode {
                "raw" => {
                    let result = scheduler
                        .socket_monitor_recvmsg_poll(
                            &mut readable,
                            &mut [IoSliceMut::new(&mut raw)],
                            SocketRecvFlags::default(),
                            &never,
                        )
                        .await
                        .unwrap();
                    assert!(!result.truncated);
                    assert_eq!(raw[0], 42);
                    std::hint::black_box(&raw[..result.bytes]);
                    result.bytes
                }
                "pool" | "shared" => {
                    let message = receiver.recv(scheduler, &never).await.unwrap().unwrap();
                    assert!(!message.result.truncated);
                    let count = message.buffer.as_ref().len();
                    assert_eq!(message.buffer.as_ref()[0], 42);
                    if mode == "shared" {
                        let frozen = message.buffer.freeze();
                        retained = Some(frozen.view(..count));
                    } else {
                        std::hint::black_box(message.buffer.as_ref());
                    }
                    count
                }
                "multi" | "multi-retain" => {
                    let message = multishot
                        .as_mut()
                        .unwrap()
                        .recv(scheduler, &never)
                        .await
                        .unwrap()
                        .unwrap();
                    assert!(!message.result.truncated);
                    let count = message.buffer.as_ref().len();
                    assert_eq!(message.buffer.as_ref()[0], 42);
                    std::hint::black_box(message.buffer.as_ref());
                    if mode == "multi-retain" {
                        retained_multi = Some(message.buffer);
                    }
                    count
                }
                _ => unreachable!("The receiver mode was validated before execution"),
            };
            assert!(count > 0);
            if udp {
                assert_eq!(count, size);
            }
            received += count;
        }
        assert_eq!(received, size * window);
    }
    let elapsed = start.elapsed();
    drop(retained);
    drop(retained_multi);
    if let Some(mut multishot) = multishot {
        multishot.close(scheduler).await.unwrap();
    }
    receiver.close(scheduler).await.unwrap();
    drop(receiver);
    scheduler
        .socket_monitor_close(&mut readable, &never)
        .await
        .unwrap();
    scheduler
        .socket_monitor_close(&mut writable, &never)
        .await
        .unwrap();
    drop((readable, writable));
    scheduler.socket_close(socket).await.unwrap();
    peer.join().unwrap();
    serde_json::json!({"transport":transport,"mode":mode,"size":size,"iterations":iterations,"seconds":elapsed.as_secs_f64(),"round_trips_per_second":iterations as f64/elapsed.as_secs_f64(),"window":window,"messages_per_second":(iterations * window) as f64 / elapsed.as_secs_f64()})
}

fn execute(args: &[String]) -> serde_json::Value {
    let backend = args.get(1).map(String::as_str).unwrap_or("native");
    let mode = args.get(2).map(String::as_str).unwrap_or("pool");
    assert!(
        matches!(mode, "raw" | "pool" | "shared" | "multi" | "multi-retain"),
        "unknown receiver mode"
    );
    let size: usize = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(65536);
    let iterations: usize = args.get(4).map(|s| s.parse().unwrap()).unwrap_or(20000);
    let workers: usize = args.get(5).map(|s| s.parse().unwrap()).unwrap_or(1);
    let transport = args.get(6).cloned().unwrap_or("tcp".into());
    assert!(
        matches!(transport.as_str(), "tcp" | "udp"),
        "unknown transport"
    );
    let window: usize = args.get(7).map(|s| s.parse().unwrap()).unwrap_or(1);
    let traffic = args.get(8).map(String::as_str).unwrap_or("echo");
    assert!(
        matches!(traffic, "echo" | "receive"),
        "unknown traffic pattern"
    );
    let receive_only = traffic == "receive";
    let mut result = match backend {
        "direct" => {
            let scheduler =
                core_scheduler_direct::DirectScheduler::new(core_scheduler_direct::DirectOptions {
                    threads: workers,
                    capacity: 64,
                })
                .unwrap();
            let result = futures_lite::future::block_on(run(
                &scheduler,
                mode,
                size,
                iterations,
                &transport,
                window,
                receive_only,
            ));
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
                .block_on(runtime.spawn(async move {
                    run(
                        owned.as_ref(),
                        &mode,
                        size,
                        iterations,
                        &transport,
                        window,
                        receive_only,
                    )
                    .await
                }))
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
                        .spawn(async move {
                            run(
                                owned.as_ref(),
                                &mode,
                                size,
                                iterations,
                                &transport,
                                window,
                                receive_only,
                            )
                            .await
                        })
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
                        .spawn(async move {
                            run(
                                owned.as_ref(),
                                &mode,
                                size,
                                iterations,
                                &transport,
                                window,
                                receive_only,
                            )
                            .await
                        })
                        .unwrap(),
                )
                .unwrap();
            scheduler.block_on(scheduler.shutdown());
            result
        }
        _ => panic!("unknown backend"),
    };
    result["traffic"] = traffic.into();
    result["backend"] = backend.into();
    result["workers"] = workers.into();
    result
}

fn main() {
    println!("{}", execute(&std::env::args().collect::<Vec<_>>()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receivers_preserve_tcp_and_udp_data_on_each_backend() {
        #[cfg(target_os = "linux")]
        let backends = ["direct", "tokio", "native", "iouring"];
        #[cfg(not(target_os = "linux"))]
        let backends = ["direct", "tokio", "native"];
        for backend in backends {
            for mode in ["raw", "pool", "shared", "multi", "multi-retain"] {
                for transport in ["tcp", "udp"] {
                    for traffic in ["echo", "receive"] {
                        let args = [
                            "sockets", backend, mode, "64", "2", "1", transport, "4", traffic,
                        ]
                        .map(str::to_owned);
                        let result = execute(&args);
                        assert_eq!(result["iterations"], 2);
                        assert_eq!(result["transport"], transport);
                        assert_eq!(result["traffic"], traffic);
                        assert_eq!(result["window"], 4);
                    }
                }
            }
        }
        assert_eq!(execute(&[])["mode"], "pool");
    }

    #[test]
    #[should_panic(expected = "unknown receiver mode")]
    fn rejects_unknown_mode() {
        execute(&["sockets", "direct", "unknown"].map(str::to_owned));
    }

    #[test]
    #[should_panic(expected = "unknown transport")]
    fn rejects_unknown_transport() {
        execute(&["sockets", "direct", "pool", "64", "2", "1", "unknown"].map(str::to_owned));
    }

    #[test]
    #[should_panic(expected = "unknown backend")]
    fn rejects_unknown_backend() {
        execute(&["sockets", "unknown"].map(str::to_owned));
    }
}
