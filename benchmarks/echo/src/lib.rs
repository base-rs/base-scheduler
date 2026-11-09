// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

mod client;
mod server;

use std::{error::Error, time::Duration};
type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

const USAGE: &str = "Usage: base-scheduler-benchmark-echo server BACKEND WORKERS CONNECTIONS\n       base-scheduler-benchmark-echo client ADDRESS WORKERS CONNECTIONS BYTES WARMUP_SECONDS MEASURE_SECONDS|requests=N [RATE]\nBackends: tokio; tokio-adapter, native and iouring, each alone (owned reads) or with a -poll, -monitor, -vectored, -owned-vectored or -pooled suffix; iouring-multishot (io_uring requires Linux). Server binds 127.0.0.1:0 and prints its address. RATE paces requests per second across all connections and measures latency from each scheduled send; without it, each connection sends its next request on receiving a response. requests=N measures until each connection has counted N requests instead, for smoke runs: its rates are not measurements.";

fn positive(value: &str) -> Result<usize> {
    let value = value.parse::<usize>()?;
    if value == 0 {
        return Err("expected a positive integer".into());
    }
    Ok(value)
}

fn seconds(value: &str) -> Result<Duration> {
    Ok(Duration::try_from_secs_f64(value.parse::<f64>()?)?)
}

// The measurement window: a positive duration in seconds, or `requests=N`.
fn measure(value: &str) -> Result<client::Measure> {
    if let Some(requests) = value.strip_prefix("requests=") {
        return Ok(client::Measure::Requests(positive(requests)? as u64));
    }
    let duration = seconds(value)?;
    if duration.is_zero() {
        return Err("use a positive measurement duration".into());
    }
    Ok(client::Measure::Seconds(duration))
}

fn runtime(workers: usize) -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()
        .expect("Tokio runtime setup")
}

/// Run a server or client using the command-line arguments, including the executable name.
pub fn run(args: &[String]) -> Result<()> {
    match args.get(1).map(String::as_str) {
        Some("server") if args.len() == 5 => {
            server::run(&args[2], positive(&args[3])?, positive(&args[4])?)
        }
        Some("client") if matches!(args.len(), 8 | 9) => {
            let connections = positive(&args[4])?;
            let bytes = positive(&args[5])?;
            let warmup = seconds(&args[6])?;
            let measure = measure(&args[7])?;
            if bytes > 65536 {
                return Err("use 1..=65536 bytes".into());
            }
            let rate = args.get(8).map(|rate| positive(rate)).transpose()?;
            runtime(positive(&args[3])?).block_on(client::run(
                args[2].parse()?,
                connections,
                bytes,
                warmup,
                measure,
                rate,
            ))
        }
        _ => Err(USAGE.into()),
    }
}
