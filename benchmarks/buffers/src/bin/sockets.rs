// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::backend::*;
use core_scheduler::*;
#[cfg(target_os = "linux")]
#[path = "../multishot.rs"]
mod multishot;
#[cfg(not(target_os = "linux"))]
use core_scheduler::backend::SocketReceiving as Multishot;
#[cfg(target_os = "linux")]
use multishot::Multishot;
use std::{
    io::{IoSliceMut, Read, Write},
    net::{TcpListener, UdpSocket},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

#[derive(Clone)]
struct Workload {
    mode: String,
    size: usize,
    iterations: usize,
    transport: String,
    window: usize,
    receive_only: bool,
    connections: usize,
    pool_layout: String,
}

struct ConnectionGroup<P> {
    pool: P,
    warming: AtomicUsize,
    ready: Cancellation,
    started: OnceLock<Instant>,
}

async fn run<S>(
    scheduler: &S,
    workload: &Workload,
    group: &ConnectionGroup<S::Pool>,
) -> serde_json::Value
where
    S: SocketOpen
        + SocketConnect
        + SocketClose
        + SocketMonitoring
        + Multishot
        + SocketSetOption<TcpNoDelay>,
    S::Socket: 'static,
{
    let mode = workload.mode.as_str();
    let size = workload.size;
    let iterations = workload.iterations;
    let transport = workload.transport.as_str();
    let window = workload.window;
    let receive_only = workload.receive_only;
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
    let mut receiver = if matches!(mode, "pool" | "shared") {
        Some(
            scheduler
                .socket_receiver(
                    &socket,
                    group.pool.clone(),
                    SocketRecvFlags::default(),
                    &never,
                )
                .await
                .unwrap(),
        )
    } else {
        None
    };
    let mut readable = scheduler
        .socket_monitor(&socket, Interest::Readable)
        .unwrap();
    let mut writable = scheduler
        .socket_monitor(&socket, Interest::Writable)
        .unwrap();
    #[cfg(target_os = "linux")]
    let mut multishot = if matches!(mode, "multi" | "multi-retain") {
        Some(
            scheduler
                .multishot(&socket, (window + 2).max(8), size, &never)
                .await
                .unwrap(),
        )
    } else {
        None
    };
    let payload = vec![42; size];
    let mut raw = vec![0; size];
    let mut retained = None;
    #[cfg(target_os = "linux")]
    let mut retained_multi = None;
    let mut receive_calls = 0_u64;
    let mut short_receives = 0_u64;
    let mut start = Instant::now();
    for index in 0..iterations + warmup {
        if index == warmup {
            receive_calls = 0;
            short_receives = 0;
            if group.warming.fetch_sub(1, Ordering::AcqRel) == 1 {
                group.started.set(Instant::now()).unwrap();
                group.ready.cancel();
            }
            group.ready.cancelled().await;
            start = *group.started.get().unwrap();
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
                    let message = receiver
                        .as_mut()
                        .unwrap()
                        .recv(scheduler, &never)
                        .await
                        .unwrap()
                        .unwrap();
                    assert!(!message.truncated());
                    let count = message.payload().len();
                    assert_eq!(message.payload()[0], 42);
                    if mode == "shared" {
                        let frozen = message.into_payload().freeze();
                        retained = Some(frozen.slice(..count));
                    } else {
                        std::hint::black_box(message.payload());
                    }
                    count
                }
                #[cfg(target_os = "linux")]
                "multi" | "multi-retain" => {
                    let message = multishot
                        .as_mut()
                        .unwrap()
                        .recv(scheduler, &never)
                        .await
                        .unwrap()
                        .unwrap();
                    assert!(!message.truncated());
                    let count = message.payload().len();
                    assert_eq!(message.payload()[0], 42);
                    std::hint::black_box(message.payload());
                    if mode == "multi-retain" {
                        retained_multi = Some(message.into_payload());
                    }
                    count
                }
                _ => unreachable!("The receiver mode was validated before execution"),
            };
            assert!(count > 0);
            receive_calls += 1;
            short_receives += u64::from(count < size);
            if udp {
                assert_eq!(count, size);
            }
            received += count;
        }
        assert_eq!(received, size * window);
    }
    let elapsed = start.elapsed();
    drop(retained);
    #[cfg(target_os = "linux")]
    drop(retained_multi);
    #[cfg(target_os = "linux")]
    if let Some(mut multishot) = multishot {
        multishot.close(scheduler).await.unwrap();
    }
    if let Some(receiver) = receiver.as_mut() {
        receiver.close(scheduler).await.unwrap();
    }
    drop(receiver);
    scheduler.socket_monitor_close(&mut readable).await.unwrap();
    scheduler.socket_monitor_close(&mut writable).await.unwrap();
    drop((readable, writable));
    scheduler.socket_close(socket).await.unwrap();
    peer.join().unwrap();
    serde_json::json!({"transport":transport,"mode":mode,"size":size,"iterations":iterations,"seconds":elapsed.as_secs_f64(),"round_trips_per_second":iterations as f64/elapsed.as_secs_f64(),"window":window,"messages_per_second":(iterations * window) as f64 / elapsed.as_secs_f64(),"receive_calls":receive_calls,"short_receives":short_receives,"bytes_received":iterations * window * size})
}

async fn run_group<S>(scheduler: Arc<S>, workload: Workload, pool: S::Pool) -> serde_json::Value
where
    S: SocketOpen
        + SocketConnect
        + SocketClose
        + SocketMonitoring
        + Multishot
        + SocketSetOption<TcpNoDelay>
        + Spawn
        + Send
        + Sync
        + 'static,
    S::Socket: 'static,
{
    assert!(workload.connections > 0);
    let capacity = pool.capacity();
    let group = Arc::new(ConnectionGroup {
        pool,
        warming: AtomicUsize::new(workload.connections),
        ready: Cancellation::new(),
        started: OnceLock::new(),
    });
    let tasks: Vec<_> = (0..workload.connections)
        .map(|_| {
            let owned = Arc::clone(&scheduler);
            let group = Arc::clone(&group);
            let workload = workload.clone();
            scheduler
                .spawn(async move { run(owned.as_ref(), &workload, &group).await })
                .unwrap()
        })
        .collect();
    let mut results = Vec::new();
    for task in tasks {
        results.push(task.await.unwrap());
    }
    let seconds = results
        .iter()
        .map(|result| result["seconds"].as_f64().unwrap())
        .fold(0.0_f64, f64::max);
    let counters = ["receive_calls", "short_receives", "bytes_received"].map(|field| {
        (
            field,
            results
                .iter()
                .map(|result| result[field].as_u64().unwrap())
                .sum::<u64>(),
        )
    });
    let mut result = results.remove(0);
    for (field, total) in counters {
        result[field] = total.into();
    }
    result["seconds"] = seconds.into();
    result["connections"] = workload.connections.into();
    result["pool_layout"] = workload.pool_layout.clone().into();
    result["pool_capacity"] = capacity.into();
    result["round_trips_per_second"] =
        (workload.connections as f64 * workload.iterations as f64 / seconds).into();
    result["messages_per_second"] =
        (workload.connections as f64 * workload.iterations as f64 * workload.window as f64
            / seconds)
            .into();
    result
}

fn execute(args: &[String]) -> serde_json::Value {
    let backend = args.get(1).map(String::as_str).unwrap_or("native");
    let mode = args.get(2).map(String::as_str).unwrap_or("pool");
    assert!(
        match mode {
            "raw" | "pool" | "shared" => true,
            #[cfg(target_os = "linux")]
            "multi" | "multi-retain" => backend == "iouring",
            _ => false,
        },
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
    let pool_layout = args.get(10).map(String::as_str).unwrap_or("total");
    assert!(
        [
            "total",
            #[cfg(target_os = "linux")]
            "worker-equal",
            #[cfg(target_os = "linux")]
            "worker-scaled"
        ]
        .contains(&pool_layout),
        "unknown pool layout"
    );
    assert!(
        backend == "iouring" || pool_layout == "total",
        "worker pool layouts require the io_uring backend"
    );
    let workload = Workload {
        mode: mode.to_owned(),
        pool_layout: pool_layout.to_owned(),
        size,
        iterations,
        transport,
        window,
        receive_only: traffic == "receive",
        connections: args.get(9).map(|s| s.parse().unwrap()).unwrap_or(1),
    };
    let options = BufferPoolOptions::new(8 * workload.connections, size);
    let mut result = match backend {
        "direct" => {
            let scheduler = Arc::new(
                core_scheduler_direct::DirectScheduler::new(core_scheduler_direct::DirectOptions {
                    threads: workers,
                    capacity: 64,
                })
                .unwrap(),
            );
            let pool = scheduler.buffer_pool(options).unwrap();
            let result =
                futures_lite::future::block_on(run_group(Arc::clone(&scheduler), workload, pool));
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
            let pool = scheduler.buffer_pool(options).unwrap();
            let result = runtime.block_on(run_group(Arc::clone(&scheduler), workload, pool));
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
            let pool = scheduler.buffer_pool(options).unwrap();
            let result = scheduler.block_on(run_group(Arc::clone(&scheduler), workload, pool));
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
            // Backend policy comparisons stay in this benchmark; the generic receive loop accepts any scheduler-created pool.
            let pool = match pool_layout {
                "total" => scheduler.buffer_pool(options),
                "worker-equal" => {
                    assert!(
                        options.buffers % workers == 0,
                        "equal budgets require a whole number of buffers per worker"
                    );
                    scheduler
                        .selector()
                        .buffer_pool_per_worker(BufferPoolOptions {
                            buffers: options.buffers / workers,
                            ..options
                        })
                }
                "worker-scaled" => scheduler.selector().buffer_pool_per_worker(options),
                _ => unreachable!("The pool layout was validated before execution"),
            }
            .unwrap();
            let result = scheduler.block_on(run_group(Arc::clone(&scheduler), workload, pool));
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
    #[should_panic(expected = "unknown receiver mode")]
    fn rejects_multishot_on_portable_backends() {
        execute(&["sockets", "tokio", "multi"].map(str::to_owned));
    }

    #[test]
    fn receivers_preserve_tcp_and_udp_data_on_each_backend() {
        #[cfg(target_os = "linux")]
        let backends = ["direct", "tokio", "native", "iouring"];
        #[cfg(not(target_os = "linux"))]
        let backends = ["direct", "tokio", "native"];
        for backend in backends {
            for mode in ["raw", "pool", "shared", "multi", "multi-retain"] {
                if mode.starts_with("multi") && backend != "iouring" {
                    continue;
                }
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
                        assert_eq!(result["bytes_received"], 2 * 4 * 64);
                        assert!(result["receive_calls"].as_u64().unwrap() >= 8);
                        if transport == "udp" {
                            assert_eq!(result["receive_calls"], 8);
                            assert_eq!(result["short_receives"], 0);
                        }
                    }
                }
            }
        }
        assert_eq!(execute(&[])["mode"], "pool");
    }

    #[test]
    fn receivers_share_capacity_across_concurrent_connections() {
        #[cfg(target_os = "linux")]
        let backends = ["direct", "tokio", "native", "iouring"];
        #[cfg(not(target_os = "linux"))]
        let backends = ["direct", "tokio", "native"];
        for backend in backends {
            for mode in ["pool", "shared"] {
                for transport in ["tcp", "udp"] {
                    let args = [
                        "sockets", backend, mode, "64", "2", "2", transport, "4", "receive", "4",
                    ]
                    .map(str::to_owned);
                    let result = execute(&args);
                    assert_eq!(result["connections"], 4);
                    assert_eq!(result["iterations"], 2);
                    assert_eq!(result["bytes_received"], 4 * 2 * 4 * 64);
                    assert!(result["receive_calls"].as_u64().unwrap() >= 32);
                    assert!(result["messages_per_second"].as_f64().unwrap() > 0.0);
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn worker_pools_preserve_data_with_equal_and_scaled_budgets() {
        for layout in ["worker-equal", "worker-scaled"] {
            for mode in ["pool", "shared"] {
                for transport in ["tcp", "udp"] {
                    let args = [
                        "sockets", "iouring", mode, "64", "2", "2", transport, "4", "receive", "4",
                        layout,
                    ]
                    .map(str::to_owned);
                    let result = execute(&args);
                    assert_eq!(
                        result["pool_capacity"],
                        if layout == "worker-equal" { 32 } else { 64 }
                    );
                    assert_eq!(result["pool_layout"], layout);
                }
            }
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    #[should_panic(expected = "worker pool layouts require the io_uring backend")]
    fn worker_pool_experiments_do_not_become_portable_requirements() {
        execute(
            &[
                "sockets",
                "native",
                "pool",
                "64",
                "2",
                "1",
                "tcp",
                "1",
                "echo",
                "1",
                "worker-equal",
            ]
            .map(str::to_owned),
        );
    }

    #[test]
    #[should_panic(expected = "unknown pool layout")]
    fn rejects_unknown_pool_layout() {
        execute(
            &[
                "sockets", "native", "pool", "64", "2", "1", "tcp", "1", "echo", "1", "unknown",
            ]
            .map(str::to_owned),
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[should_panic(expected = "equal budgets require")]
    fn rejects_unequal_worker_budgets() {
        execute(
            &[
                "sockets",
                "iouring",
                "pool",
                "64",
                "2",
                "3",
                "tcp",
                "1",
                "echo",
                "1",
                "worker-equal",
            ]
            .map(str::to_owned),
        );
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
