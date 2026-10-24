// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

mod client;
mod server;

use std::{error::Error, time::Duration};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Clone, Copy)]
struct Workload {
    bytes: usize,
    chunks: usize,
    work: usize,
}

impl Workload {
    fn parse(args: &[String]) -> Result<Self> {
        let bytes = positive(&args[0])?;
        let chunks = positive(&args[1])?;
        let work = args[2].parse()?;
        if bytes > 1024 * 1024 || chunks > 1024 || work > 4096 {
            return Err("maximum bytes/chunks/work: 1048576/1024/4096".into());
        }
        Ok(Self {
            bytes,
            chunks,
            work,
        })
    }
}

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

fn runtime(workers: usize) -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()
        .expect("Tokio runtime setup")
}

// Fill the actual output allocation. `work` controls a dependency chain per cache line; its result affects every byte, so the compiler cannot discard it. Chunk-specific contents catch reordering, duplication, and partially overwritten leases. Clients precompute expected contents outside the timed loop.
// Keep one shared implementation across the differently shaped backend futures so payload generation is not a separate compiler-inlining experiment.
#[inline(never)]
fn prepare(buffer: &mut [u8], chunk: usize, work: usize) {
    let mut state = (chunk as u64).wrapping_add(0x9e3779b97f4a7c15);
    for line in buffer.chunks_mut(64) {
        for _ in 0..work {
            state = state.rotate_left(17).wrapping_mul(0xd6e8feb86659fd93);
        }
        state = state.wrapping_add(0xa0761d6478bd642f);
        for word in line.chunks_mut(8) {
            let value = state.to_le_bytes();
            word.copy_from_slice(&value[..word.len()]);
            state = state.wrapping_add(0xe7037ed1a0b428db);
        }
    }
}

/// Execute the write benchmark server or client. Both processes receive the same payload, chunk, and generation-work arguments.
pub fn run(args: &[String]) -> Result<()> {
    match args.get(1).map(String::as_str) {
        Some("server") if args.len() == 9 => server::run(
            &args[2],
            positive(&args[3])?,
            positive(&args[4])?,
            Workload::parse(&args[5..8])?,
            args[8].parse()?,
        ),
        Some("client") if args.len() == 10 => {
            let duration = seconds(&args[9])?;
            if duration.is_zero() {
                return Err("measurement duration must be positive".into());
            }
            runtime(positive(&args[3])?).block_on(client::run(
                args[2].parse()?,
                positive(&args[4])?,
                Workload::parse(&args[5..8])?,
                seconds(&args[8])?,
                duration,
            ))
        }
        _ => Err("Usage: core-scheduler-write-benchmark server BACKEND WORKERS CONNECTIONS BYTES CHUNKS WORK SEND_BUFFER\n       core-scheduler-write-benchmark client ADDRESS WORKERS CONNECTIONS BYTES CHUNKS WORK WARMUP_SECONDS MEASURE_SECONDS\nBackends: tokio, tokio-monitor, tokio-writer, native-monitor, iouring-monitor, iouring-writer. SEND_BUFFER=0 uses the OS default; otherwise it requests SO_SNDBUF bytes. Native backends require Linux.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_payload_distinguishes_chunks_and_work() {
        let mut first = [0; 137];
        let mut second = [0; 137];
        prepare(&mut first, 0, 0);
        prepare(&mut second, 1, 0);
        assert_ne!(first, second);
        prepare(&mut second, 0, 1);
        assert_ne!(first, second);
        prepare(&mut second, 0, 0);
        assert_eq!(first, second);
    }

    #[test]
    fn invalid_cli_arguments_fail() {
        assert!(positive("0").is_err());
        assert!(positive("n").is_err());
        assert!(seconds("-1").is_err());
        assert!(run(&["benchmark".into()]).is_err());
        for values in [
            ["0", "1", "0"],
            ["1048577", "1", "0"],
            ["1", "1025", "0"],
            ["1", "1", "4097"],
        ] {
            assert!(Workload::parse(&values.map(str::to_owned)).is_err());
        }
    }
}
