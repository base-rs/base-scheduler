// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::Result;
use hdrhistogram::Histogram;
use std::{
    io,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{Barrier, watch},
};

// Tokio timers fire on the first millisecond tick after their deadline, which would bunch paced requests. Sleep until just over one tick before the scheduled instant, then yield until it. The yield loop keeps a client worker busy for about this long before each paced send, so pin the client to CPUs apart from the server.
const PACE_SPIN: Duration = Duration::from_micros(1250);

async fn pace(until: Instant) {
    let coarse = until - PACE_SPIN;
    if coarse > Instant::now() {
        tokio::time::sleep_until(coarse.into()).await;
    }
    while Instant::now() < until {
        tokio::task::yield_now().await;
    }
}

pub(crate) async fn run(
    address: SocketAddr,
    connections: usize,
    bytes: usize,
    warmup: Duration,
    duration: Duration,
    rate: Option<usize>,
) -> Result<()> {
    // Each connection sends at an equal share of the offered rate.
    let interval = rate.map(|rate| Duration::from_secs_f64(connections as f64 / rate as f64));
    if interval.is_some_and(|interval| interval.is_zero()) {
        return Err("offered rate is too high to pace".into());
    }
    let gate = Arc::new(Barrier::new(connections + 1));
    let (schedule, clock) = watch::channel(None::<(Instant, Instant)>);
    let mut tasks = Vec::with_capacity(connections);
    for index in 0..connections {
        let mut socket = TcpStream::connect(address)
            .await
            .expect("connect benchmark client");
        socket.set_nodelay(true).expect("TCP_NODELAY");
        let gate = Arc::clone(&gate);
        let mut clock = clock.clone();
        tasks.push(tokio::spawn(async move {
            let request: Vec<u8> = (0..bytes)
                .map(|offset| (offset.wrapping_add(index) % 251) as u8)
                .collect();
            let mut response = vec![0; bytes];
            // Reserve histogram storage before warmup; recording needs no allocation in the timed loop.
            let mut latency = Histogram::<u64>::new_with_bounds(1, 60_000_000_000, 3)
                .expect("valid histogram bounds");
            gate.wait().await;
            clock
                .changed()
                .await
                .expect("coordinator publishes schedule");
            let (measure_start, measure_end) =
                clock.borrow().expect("coordinator publishes the schedule");
            // Paced connections start at staggered offsets from the shared schedule, so their requests do not arrive together.
            let base = measure_start - warmup;
            let mut scheduled =
                interval.map(|interval| base + interval.mul_f64(index as f64 / connections as f64));
            // The longest any counted paced request waited past its scheduled send: near zero unless the server or client fell behind.
            let mut send_delay = Duration::ZERO;
            loop {
                // A paced request's latency starts at its scheduled send, including any delay before it could be sent.
                let started = match scheduled.as_mut().zip(interval) {
                    Some((scheduled, interval)) => {
                        if *scheduled >= measure_end {
                            break;
                        }
                        let started = *scheduled;
                        *scheduled += interval;
                        pace(started).await;
                        started
                    }
                    None => Instant::now(),
                };
                // Stop at the end of the window in wall-clock time, even if an overloaded server has left paced requests unsent.
                let sent = Instant::now();
                if started >= measure_end || sent >= measure_end {
                    break;
                }
                socket.write_all(&request).await.expect("client write");
                socket.read_exact(&mut response).await.expect("client read");
                let completed = Instant::now();
                if response != request {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "echo payload mismatch",
                    )
                    .into());
                }
                // Count round trips sent and completed inside the shared measurement window; warmup and boundary-crossing requests are excluded. A paced request is counted by when it was actually sent, so an overloaded server's backlog still counts, with its queueing delay in the latency.
                if sent >= measure_start && completed <= measure_end {
                    send_delay = send_delay.max(sent - started);
                    latency
                        .record((completed - started).as_nanos().max(1) as u64)
                        .expect("round trip within histogram bounds");
                }
            }
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>((latency, send_delay))
        }));
    }
    gate.wait().await;
    let measure_start = Instant::now() + warmup;
    schedule
        .send(Some((measure_start, measure_start + duration)))
        .expect("clients await schedule");
    let mut latency =
        Histogram::<u64>::new_with_bounds(1, 60_000_000_000, 3).expect("valid histogram bounds");
    let mut send_delay = Duration::ZERO;
    for task in tasks {
        let (connection, connection_delay) = task.await.expect("client task failed")?;
        latency.add(connection).expect("matching histogram bounds");
        send_delay = send_delay.max(connection_delay);
    }
    if latency.is_empty() {
        return Err("no complete requests in measurement window".into());
    }
    println!(
        "{}",
        serde_json::json!({
            "connections": connections, "bytes": bytes, "offered_requests_per_second": rate, "paced_send_delay_max_us": rate.map(|_| send_delay.as_secs_f64() * 1e6), "warmup_seconds": warmup.as_secs_f64(), "seconds": duration.as_secs_f64(),
            "requests": latency.len(), "requests_per_second": latency.len() as f64 / duration.as_secs_f64(),
            "payload_mib_per_second": latency.len() as f64 * bytes as f64 / duration.as_secs_f64() / 1048576.0,
            "rtt_mean_us": latency.mean() / 1000.0, "rtt_p50_us": latency.value_at_quantile(0.50) as f64 / 1000.0,
            "rtt_p95_us": latency.value_at_quantile(0.95) as f64 / 1000.0, "rtt_p99_us": latency.value_at_quantile(0.99) as f64 / 1000.0,
            "rtt_max_us": latency.max() as f64 / 1000.0,
        })
    );
    Ok(())
}
