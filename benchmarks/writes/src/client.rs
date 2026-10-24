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

pub(crate) async fn run(
    address: SocketAddr,
    connections: usize,
    workload: Workload,
    warmup: Duration,
    duration: Duration,
) -> Result<()> {
    let gate = Arc::new(Barrier::new(connections + 1));
    let (schedule, clock) = watch::channel(None::<(Instant, Instant)>);
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
        )));
    }
    gate.wait().await;
    let start = Instant::now() + warmup;
    schedule
        .send(Some((start, start + duration)))
        .expect("clients await schedule");
    let mut latency =
        Histogram::<u64>::new_with_bounds(1, 60_000_000_000, 3).expect("valid histogram bounds");
    for task in tasks {
        latency
            .add(task.await.expect("client task completed without panic")?)
            .expect("matching histogram bounds");
    }
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
    mut clock: watch::Receiver<Option<(Instant, Instant)>>,
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
        let started = Instant::now();
        if started >= end {
            break;
        }
        socket.write_all(&[(request + 1) as u8]).await?;
        for expected in &expected[request] {
            socket.read_exact(&mut response).await?;
            if response != *expected {
                return Err("write payload mismatch".into());
            }
        }
        let completed = Instant::now();
        request ^= 1;
        if started >= start && completed <= end {
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
                Duration::from_secs(1),
            ))
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "client TCP_NODELAY");
    }

    #[test]
    fn failed_request_writes_return_the_transport_error() {
        let runtime = crate::runtime(1);
        runtime.block_on(async {
            let (socket, peer) = tokio::io::duplex(1);
            drop(peer);
            let (schedule, clock) = watch::channel(None);
            let start = Instant::now();
            schedule
                .send(Some((start, start + Duration::from_secs(1))))
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
