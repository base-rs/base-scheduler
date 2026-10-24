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
    io::{AsyncReadExt, AsyncWriteExt},
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
        let mut socket = TcpStream::connect(address).await?;
        socket.set_nodelay(true)?;
        let gate = Arc::clone(&gate);
        let expected = Arc::clone(&expected);
        let mut clock = clock.clone();
        tasks.push(tokio::spawn(async move {
            let mut response = vec![0; workload.bytes];
            let mut latency = Histogram::<u64>::new_with_bounds(1, 60_000_000_000, 3)?;
            gate.wait().await;
            clock.changed().await?;
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
                    latency.record((completed - started).as_nanos().max(1) as u64)?;
                }
            }
            socket.shutdown().await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(latency)
        }));
    }
    gate.wait().await;
    let start = Instant::now() + warmup;
    schedule.send(Some((start, start + duration)))?;
    let mut latency = Histogram::<u64>::new_with_bounds(1, 60_000_000_000, 3)?;
    for task in tasks {
        latency.add(task.await??)?;
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
