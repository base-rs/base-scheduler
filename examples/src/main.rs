// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::*;
use core_scheduler_examples::{datagrams, files, frames, stream};
use std::{
    collections::VecDeque,
    io::Write,
    net::{TcpListener, UdpSocket},
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Workload {
    recipe: String,
    size: usize,
    count: usize,
    buffer_size: usize,
}

impl Workload {
    fn data(&self) -> Vec<u8> {
        let mut data: Vec<u8> = (0..self.size * self.count)
            .map(|index| (index % 251) as u8)
            .collect();
        if self.recipe == "datagrams" {
            for (index, packet) in data.chunks_mut(self.size).enumerate() {
                packet[..8].copy_from_slice(&(index as u64).to_le_bytes());
            }
        }
        data
    }
}

async fn network<S>(scheduler: &S, workload: &Workload) -> Duration
where
    S: SocketScheduler + SocketMonitoring + SocketReceiving + Spawn,
    S::Socket: 'static,
{
    let data = Arc::new(workload.data());
    let outgoing = Arc::clone(&data);
    let (start, started) = mpsc::channel();
    let cancellation = Cancellation::new();
    let timeout = cancellation.clone();
    let size = workload.size;
    let packets = workload.count;
    let (acknowledge, acknowledged) = mpsc::channel();
    let (address, peer) = if workload.recipe == "datagrams" {
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        sender
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let address = sender.local_addr().unwrap();
        let peer = std::thread::spawn(move || {
            // Discover the receiver, then bound the sender's window so this test does not measure UDP loss.
            let (_, remote) = sender.recv_from(&mut [0]).unwrap();
            started.recv().unwrap();
            let result = send_packets(&sender, remote, &outgoing, size, &acknowledged);
            // End any receive wait if the peer stalls; on success every packet has already been processed.
            timeout.cancel();
            result.unwrap();
        });
        (address, peer)
    } else {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let peer = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            started.recv().unwrap();
            socket.write_all(&outgoing).unwrap();
        });
        (address, peer)
    };
    let socket = scheduler
        .socket_open(
            SocketDomain::Ipv4,
            if workload.recipe == "datagrams" {
                SocketType::Datagram
            } else {
                SocketType::Stream
            },
            None,
            &cancellation,
        )
        .await
        .unwrap();
    scheduler
        .socket_connect(&socket, address, &cancellation)
        .await
        .unwrap();
    // Allocation and socket setup precede the timed recipe. The budget allows retained frames or eight packet handlers plus reception.
    let pool = scheduler
        .buffer_pool(BufferPoolOptions::new(16, workload.buffer_size))
        .unwrap();
    let mut buffer = vec![0; workload.buffer_size];
    if workload.recipe == "datagrams" {
        scheduler
            .socket_send_poll(&socket, &[0], SocketSendFlags::default(), &cancellation)
            .await
            .unwrap();
    }
    let mut retained = VecDeque::new();
    let mut position = 0;
    let began = Instant::now();
    start.send(()).unwrap();
    match workload.recipe.as_str() {
        "stream" => {
            let received =
                stream::receive(scheduler, &socket, &mut buffer, &cancellation, |bytes| {
                    assert_eq!(bytes, &data[position..position + bytes.len()]);
                    position += bytes.len();
                    Ok(())
                })
                .await
                .unwrap();
            assert_eq!(received, data.len());
        }
        "frames" => {
            frames::receive(scheduler, &socket, pool, size, &cancellation, |frame| {
                for (offset, retained) in &retained {
                    assert_eq!(
                        frames::Frame::as_ref(retained),
                        &data[*offset..*offset + size]
                    );
                }
                assert_eq!(frame.as_ref(), &data[position..position + size]);
                retained.push_back((position, frame));
                position += size;
                // Keep views alive over subsequent receives without monopolizing the pool.
                if retained.len() > 2 {
                    std::hint::black_box(retained.pop_front());
                }
                Ok(())
            })
            .await
            .unwrap();
            assert_eq!(position, data.len());
        }
        "datagrams" => {
            let received = Arc::new(std::sync::Mutex::new(vec![false; packets]));
            let seen = Arc::clone(&received);
            let packet_data = Arc::clone(&data);
            datagrams::receive(
                scheduler,
                &socket,
                pool,
                packets,
                8,
                &cancellation,
                move |message| {
                    assert_eq!(message.address().unwrap().to_socket_address(), address);
                    // Payloads contain a sequence number so reordering is valid but loss and duplication are detected.
                    let index =
                        u64::from_le_bytes(message.payload()[..8].try_into().unwrap()) as usize;
                    assert_eq!(
                        message.payload(),
                        &packet_data[index * size..(index + 1) * size]
                    );
                    assert!(!std::mem::replace(&mut seen.lock().unwrap()[index], true));
                    acknowledge.send(()).unwrap();
                    Ok(())
                },
            )
            .await
            .unwrap();
            assert!(received.lock().unwrap().iter().all(|received| *received));
        }
        _ => unreachable!("The recipe is validated before network setup"),
    }
    let elapsed = began.elapsed();
    // These owners deliberately survive receiver shutdown.
    for (offset, frame) in retained {
        assert_eq!(frame.as_ref(), &data[offset..offset + size]);
    }
    scheduler.socket_close(socket).await.unwrap();
    peer.join().unwrap();
    elapsed
}

fn send_packets(
    sender: &UdpSocket,
    remote: std::net::SocketAddr,
    data: &[u8],
    size: usize,
    acknowledged: &mpsc::Receiver<()>,
) -> Result<(), mpsc::RecvTimeoutError> {
    let packets = data.len() / size;
    for batch in (0..packets).step_by(8) {
        let end = (batch + 8).min(packets);
        for index in batch..end {
            sender
                .send_to(&data[index * size..(index + 1) * size], remote)
                .unwrap();
        }
        for _ in batch..end {
            acknowledged.recv_timeout(Duration::from_secs(10))?;
        }
    }
    Ok(())
}

async fn file<S: FileOpen + FileClose + FileMonitoring>(
    scheduler: &S,
    workload: &Workload,
) -> Duration {
    let directory = tempfile::tempdir().unwrap();
    let input_path = directory.path().join("input");
    let output_path = directory.path().join("output");
    let data = workload.data();
    std::fs::write(&input_path, &data).unwrap();
    std::fs::write(&output_path, vec![0; data.len()]).unwrap();
    let never = Cancellation::never();
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
    let buffers = [vec![0; workload.buffer_size], vec![0; workload.buffer_size]];
    let began = Instant::now();
    let copied = files::copy(scheduler, &input, &output, buffers, &never)
        .await
        .unwrap();
    let elapsed = began.elapsed();
    scheduler.file_close(input).await.unwrap();
    scheduler.file_close(output).await.unwrap();
    assert_eq!(copied, data.len());
    assert_eq!(std::fs::read(output_path).unwrap(), data);
    elapsed
}

async fn measure<S>(scheduler: Arc<S>, workload: Workload) -> f64
where
    S: SocketScheduler
        + SocketMonitoring
        + SocketReceiving
        + Spawn
        + FileOpen
        + FileClose
        + FileMonitoring
        + Send
        + Sync
        + 'static,
    S::Socket: 'static,
{
    let owned = Arc::clone(&scheduler);
    scheduler
        .spawn(async move {
            let mut elapsed = Duration::ZERO;
            // A complete untimed pass warms the runtime and page cache. The second pass is the sample.
            for _ in 0..2 {
                elapsed = if workload.recipe == "files" {
                    file(owned.as_ref(), &workload).await
                } else {
                    network(owned.as_ref(), &workload).await
                };
            }
            elapsed.as_secs_f64()
        })
        .unwrap()
        .await
        .unwrap()
}

fn execute(args: &[String]) -> serde_json::Value {
    let backend = args.first().map(String::as_str).unwrap_or("native");
    let recipe = args.get(1).map(String::as_str).unwrap_or("stream");
    assert!(
        ["stream", "frames", "datagrams", "files"].contains(&recipe),
        "unknown recipe"
    );
    let size = args
        .get(2)
        .map(|value| value.parse().unwrap())
        .unwrap_or(1024);
    let count = args
        .get(3)
        .map(|value| value.parse().unwrap())
        .unwrap_or(1024);
    let workers = args.get(4).map(|value| value.parse().unwrap()).unwrap_or(1);
    let buffer_size = args
        .get(5)
        .map(|value| value.parse().unwrap())
        .unwrap_or(65536);
    assert!(
        size > 0 && count > 0 && workers > 0 && buffer_size > 0,
        "sizes and workers must be positive"
    );
    assert!(
        recipe != "datagrams" || (8..=65507).contains(&size) && size <= buffer_size,
        "datagram size must fit its sequence number, storage, and UDP"
    );
    let workload = Workload {
        recipe: recipe.to_owned(),
        size,
        count,
        buffer_size,
    };
    let seconds = match backend {
        "direct" => {
            let scheduler = Arc::new(
                core_scheduler_direct::DirectScheduler::new(core_scheduler_direct::DirectOptions {
                    threads: workers,
                    capacity: 64,
                })
                .unwrap(),
            );
            let result = futures_lite::future::block_on(measure(Arc::clone(&scheduler), workload));
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
            let result = runtime.block_on(measure(Arc::clone(&scheduler), workload));
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
            let result = scheduler.block_on(measure(Arc::clone(&scheduler), workload));
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
            let result = scheduler.block_on(measure(Arc::clone(&scheduler), workload));
            scheduler.block_on(scheduler.shutdown());
            result
        }
        _ => panic!("unknown backend"),
    };
    serde_json::json!({"backend": backend, "recipe": recipe, "size": size, "count": count, "workers": workers, "buffer_size": buffer_size, "bytes": size * count, "seconds": seconds, "mib_per_second": (size * count) as f64 / seconds / 1048576.0})
}

fn main() {
    println!("{}", execute(&std::env::args().skip(1).collect::<Vec<_>>()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopped_packet_consumer_ends_the_generator() {
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
        let (acknowledge, acknowledged) = mpsc::channel();
        drop(acknowledge);
        assert!(
            send_packets(
                &sender,
                peer.local_addr().unwrap(),
                &[42; 16],
                16,
                &acknowledged
            )
            .is_err()
        );
    }

    #[test]
    fn recipes_preserve_data_on_every_backend() {
        for backend in [
            "direct",
            "tokio",
            "native",
            #[cfg(target_os = "linux")]
            "iouring",
        ] {
            for recipe in ["stream", "frames", "datagrams", "files"] {
                for workers in ["1", "2"] {
                    let result = execute(
                        &[backend, recipe, "257", "19", workers, "1024"].map(str::to_owned),
                    );
                    assert_eq!(result["bytes"], 257 * 19);
                    assert_eq!(result["backend"], backend);
                }
            }
        }
        assert_eq!(execute(&[])["recipe"], "stream");
    }

    #[test]
    #[should_panic(expected = "unknown recipe")]
    fn rejects_unknown_recipe() {
        execute(&["native", "unknown"].map(str::to_owned));
    }

    #[test]
    #[should_panic(expected = "unknown backend")]
    fn rejects_unknown_backend() {
        execute(&["unknown"].map(str::to_owned));
    }

    #[test]
    #[should_panic(expected = "sizes and workers must be positive")]
    fn rejects_empty_storage() {
        execute(&["native", "stream", "0"].map(str::to_owned));
    }

    #[test]
    #[should_panic(expected = "datagram size must fit")]
    fn rejects_datagram_overflow() {
        execute(&["native", "datagrams", "65508"].map(str::to_owned));
    }
}
