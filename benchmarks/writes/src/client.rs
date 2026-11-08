// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{Result, Workload, prepare};
use hdrhistogram::Histogram;
use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    sync::{Barrier, watch},
};

/// How long the measurement window lasts, from the end of the warmup.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Measure {
    /// A fixed duration: the benchmark's measurement.
    Seconds(Duration),
    /// Until each connection has counted this many requests, however long that takes, so that a smoke run cannot come out empty on a slow host. Its rates are not measurements.
    Requests(u64),
}

// Where the window ends, published to the connections with its start.
#[derive(Clone, Copy, Debug)]
enum End {
    At(Instant),
    After(u64),
}

impl End {
    // Whether a connection that has counted `counted` requests stops before sending at `now`.
    fn reached(self, counted: u64, now: Instant) -> bool {
        match self {
            Self::At(end) => now >= end,
            Self::After(requests) => counted >= requests,
        }
    }

    // Whether a request completed at `completed` finished inside the window.
    fn includes(self, completed: Instant) -> bool {
        match self {
            Self::At(end) => completed <= end,
            Self::After(_) => true,
        }
    }
}

pub(crate) async fn run(
    address: SocketAddr,
    connections: usize,
    workload: Workload,
    warmup: Duration,
    measure: Measure,
) -> Result<()> {
    let gate = Arc::new(Barrier::new(connections + 1));
    let (schedule, clock) = watch::channel(None::<(Instant, End)>);
    let expected: Arc<Vec<Vec<Vec<u8>>>> = Arc::new(
        (1..=2)
            .map(|request| {
                (0..workload.chunks)
                    .map(|chunk| {
                        let mut buffer = vec![0; workload.bytes];
                        prepare(
                            &mut buffer,
                            chunk + request * workload.chunks,
                            workload.work,
                        );
                        buffer
                    })
                    .collect()
            })
            .collect(),
    );
    let mut tasks = Vec::with_capacity(connections);
    for _ in 0..connections {
        let socket = TcpStream::connect(address).await?;
        crate::setup("client TCP_NODELAY", || socket.set_nodelay(true))?;
        tasks.push(tokio::spawn(connection(
            socket,
            workload,
            Arc::clone(&expected),
            Arc::clone(&gate),
            clock.clone(),
            Instant::now,
        )));
    }
    gate.wait().await;
    let start = Instant::now() + warmup;
    let end = match measure {
        Measure::Seconds(duration) => End::At(start + duration),
        Measure::Requests(requests) => End::After(requests),
    };
    schedule
        .send(Some((start, end)))
        .expect("clients await schedule");
    let mut latency =
        Histogram::<u64>::new_with_bounds(1, 60_000_000_000, 3).expect("valid histogram bounds");
    for task in tasks {
        latency
            .add(task.await.expect("client task completed without panic")?)
            .expect("matching histogram bounds");
    }
    // A window of requests ends when its last connection does.
    let duration = match measure {
        Measure::Seconds(duration) => duration,
        Measure::Requests(_) => start.elapsed(),
    };
    if latency.is_empty() {
        return Err("no complete responses in measurement window".into());
    }
    println!(
        "{}",
        serde_json::json!({
            "connections": connections, "bytes": workload.bytes, "chunks": workload.chunks, "work": workload.work,
            "warmup_seconds": warmup.as_secs_f64(), "seconds": duration.as_secs_f64(),
            "requests": latency.len(), "requests_per_second": latency.len() as f64 / duration.as_secs_f64(),
            "payload_mib_per_second": latency.len() as f64 * workload.bytes as f64 * workload.chunks as f64 / duration.as_secs_f64() / 1048576.0,
            "rtt_mean_us": latency.mean() / 1000.0, "rtt_p50_us": latency.value_at_quantile(0.50) as f64 / 1000.0,
            "rtt_p95_us": latency.value_at_quantile(0.95) as f64 / 1000.0, "rtt_p99_us": latency.value_at_quantile(0.99) as f64 / 1000.0,
            "rtt_max_us": latency.max() as f64 / 1000.0,
        })
    );
    Ok(())
}

async fn connection(
    mut socket: impl AsyncRead + AsyncWrite + Unpin,
    workload: Workload,
    expected: Arc<Vec<Vec<Vec<u8>>>>,
    gate: Arc<Barrier>,
    mut clock: watch::Receiver<Option<(Instant, End)>>,
    mut now: impl FnMut() -> Instant,
) -> Result<Histogram<u64>> {
    let mut response = vec![0; workload.bytes];
    let mut latency =
        Histogram::<u64>::new_with_bounds(1, 60_000_000_000, 3).expect("valid histogram bounds");
    gate.wait().await;
    clock
        .changed()
        .await
        .expect("coordinator publishes schedule");
    let (start, end) = clock.borrow().expect("coordinator publishes schedule");
    let mut request = 0;
    loop {
        let started = now();
        if end.reached(latency.len(), started) {
            break;
        }
        socket.write_all(&[(request + 1) as u8]).await?;
        for expected in &expected[request] {
            socket.read_exact(&mut response).await?;
            if response != *expected {
                return Err("write payload mismatch".into());
            }
        }
        let completed = now();
        request ^= 1;
        if started >= start && end.includes(completed) {
            latency
                .record((completed - started).as_nanos().max(1) as u64)
                .expect("round trip within histogram bounds");
        }
    }
    // Dropping the task-owned socket sends EOF after the final response.
    Ok::<_, Box<dyn std::error::Error + Send + Sync>>(latency)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io, net::TcpListener};

    #[test]
    fn failed_socket_configuration_is_reported() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let runtime = crate::runtime(1);
        let error = crate::tests::fail_setup("client TCP_NODELAY", || {
            runtime.block_on(run(
                address,
                1,
                Workload {
                    bytes: 1,
                    chunks: 1,
                    work: 0,
                },
                Duration::ZERO,
                Measure::Seconds(Duration::from_secs(1)),
            ))
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "client TCP_NODELAY");
    }

    #[test]
    fn responses_must_start_and_finish_inside_the_measurement_window() {
        let runtime = crate::runtime(1);
        runtime.block_on(async {
            let start = Instant::now();
            let end = start + Duration::from_secs(1);
            for (started, completed, count) in [
                (start - Duration::from_nanos(1), start, 0),
                (start, end, 1),
                (start, end + Duration::from_nanos(1), 0),
            ] {
                let (socket, mut peer) = tokio::io::duplex(1);
                let server = tokio::spawn(async move {
                    let mut request = [0];
                    peer.read_exact(&mut request).await.unwrap();
                    assert_eq!(request, [1]);
                    peer.write_all(&[69]).await.unwrap();
                    assert_eq!(peer.read(&mut request).await.unwrap(), 0);
                });
                let (schedule, clock) = watch::channel(None);
                schedule.send(Some((start, End::At(end)))).unwrap();
                // Script the round-trip times so preemption cannot change which side of the measurement boundary the response falls on.
                let mut times = [started, completed, end].into_iter();
                let latency = connection(
                    socket,
                    Workload {
                        bytes: 1,
                        chunks: 1,
                        work: 0,
                    },
                    Arc::new(vec![vec![vec![69]]; 2]),
                    Arc::new(Barrier::new(1)),
                    clock,
                    || times.next().expect("one round trip and the exit check"),
                )
                .await
                .unwrap();
                server.await.unwrap();
                assert_eq!(latency.len(), count);
                assert!(times.next().is_none());
            }
        });
    }

    #[test]
    fn a_window_of_requests_counts_responses_after_the_warmup_until_each_connection_has_its_count()
    {
        let runtime = crate::runtime(1);
        runtime.block_on(async {
            let (socket, mut peer) = tokio::io::duplex(1);
            let server = tokio::spawn(async move {
                for expected in [1, 2] {
                    let mut request = [0];
                    peer.read_exact(&mut request).await.unwrap();
                    assert_eq!(request, [expected]);
                    peer.write_all(&[69]).await.unwrap();
                }
                assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
            });
            let start = Instant::now();
            let (schedule, clock) = watch::channel(None);
            schedule.send(Some((start, End::After(1)))).unwrap();
            // The first round trip starts in the warmup, so it is not counted; the second is, however late it completes, and the count then ends the window.
            let late = start + Duration::from_secs(30);
            let mut times = [start - Duration::from_nanos(1), start, start, late, late].into_iter();
            let latency = connection(
                socket,
                Workload {
                    bytes: 1,
                    chunks: 1,
                    work: 0,
                },
                Arc::new(vec![vec![vec![69]]; 2]),
                Arc::new(Barrier::new(1)),
                clock,
                || times.next().expect("two round trips and the exit check"),
            )
            .await
            .unwrap();
            server.await.unwrap();
            assert_eq!(latency.len(), 1);
            assert_eq!(latency.max(), latency.highest_equivalent(30_000_000_000));
            assert!(times.next().is_none());
        });
    }

    #[test]
    fn an_empty_measurement_is_reported() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let runtime = crate::runtime(1);
        let error = runtime
            .block_on(run(
                listener.local_addr().unwrap(),
                1,
                Workload {
                    bytes: 1,
                    chunks: 1,
                    work: 0,
                },
                Duration::ZERO,
                Measure::Seconds(Duration::ZERO),
            ))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "no complete responses in measurement window"
        );
    }

    #[test]
    fn failed_request_writes_return_the_transport_error() {
        let runtime = crate::runtime(1);
        runtime.block_on(async {
            let (socket, peer) = tokio::io::duplex(1);
            drop(peer);
            let (schedule, clock) = watch::channel(None);
            // A window of requests, so that no slowness can end it before the first write.
            schedule
                .send(Some((Instant::now(), End::After(1))))
                .unwrap();
            let error = connection(
                socket,
                Workload {
                    bytes: 1,
                    chunks: 1,
                    work: 0,
                },
                Arc::new(vec![vec![vec![0]]; 2]),
                Arc::new(Barrier::new(1)),
                clock,
                Instant::now,
            )
            .await
            .unwrap_err();
            assert_eq!(
                error.downcast::<io::Error>().unwrap().kind(),
                io::ErrorKind::BrokenPipe
            );
        });
    }
}
