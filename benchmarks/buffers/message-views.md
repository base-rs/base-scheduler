# Received-message accessor comparison

The received-message API now exposes backend-associated address and payload views instead of a fixed metadata record. io_uring multishot messages retain their native source-address range and convert it only when requested. This comparison checks normal payload consumption; the timed loops do not inspect or materialize source addresses, so it does not measure address-heavy routing or repeated address conversion.

## Method

The [784 samples](results/message-views.jsonl) compare baseline `18776a85125ea922b773a90aaffd3c6b439d0f8a` with the implementation in `5f33880c97af0a659a64c9a9be06f0bd0518d161` plus declaration grouping in `src/lib.rs`. The grouping has no behavioral change. Both binaries use Cargo’s optimized release profile. Each result records its executable SHA-256:

- Before: `571830981304053ab67a0bc551ac5d7027290b84df106d034346a007c91cc838`.
- After: `08e1fad9d2ff750e107c7680e7054944d408842e83d62e43325705de3a20b261`.

Hana uses an Intel Xeon E3-1240 v6 with four physical cores and eight hardware threads, Linux 7.2.3-arch1-2, and Rust 1.98.1. Processes have CPU affinity 2–5 without individual thread pinning. Runs hold `/tmp/core-scheduler-perf.lock`, excluding overlapping cooperating builds, tests, and benchmarks. Seven matched repetitions use randomized case and before/after ordering with seed 232810, one connection, eight pool slots where applicable, 1,000 warmup batches, and 3,000 measured batches.

`raw` is the borrowed monitor control. `pool` uses the ordinary demand-driven shared pool. `multi` uses the legacy optional multishot capability, emulated by polling on Tokio. `provided` is io_uring’s concrete worker-owned provided-buffer pool with `worker-equal` sizing; it is separate from the portable pool policy. The comparisons are loopback receive workloads without parsing, encryption, physical-network traffic, or long-lived parser views.

## Results

Cells show the median percentage change in messages per second from matched after/before ratios; positive values favor the new API.

### Tokio

| Workload / window | Workers | raw | pool | multi |
| --- | ---: | ---: | ---: | ---: |
| TCP 64 B / 1 | 1 | -0.1% | -2.4% | -0.4% |
| TCP 64 B / 1 | 2 | -1.2% | +0.5% | +1.5% |
| TCP 64 B / 16 | 1 | -0.2% | -0.1% | +0.7% |
| TCP 64 B / 16 | 2 | -0.4% | +0.1% | +0.1% |
| TCP 64 KiB / 4 | 1 | +1.2% | +0.3% | -0.0% |
| TCP 64 KiB / 4 | 2 | -1.4% | -0.6% | +0.0% |
| UDP 1200 B / 16 | 1 | -1.0% | +0.2% | -0.1% |
| UDP 1200 B / 16 | 2 | -0.5% | +1.7% | -0.4% |

### io_uring

| Workload / window | Workers | raw | pool | multi | provided |
| --- | ---: | ---: | ---: | ---: | ---: |
| TCP 64 B / 1 | 1 | -1.8% | -1.6% | -3.9% | +2.5% |
| TCP 64 B / 1 | 2 | +0.0% | -0.4% | +2.8% | +1.3% |
| TCP 64 B / 16 | 1 | +0.3% | -0.1% | -0.6% | +0.4% |
| TCP 64 B / 16 | 2 | +0.9% | -0.6% | +0.8% | +0.5% |
| TCP 64 KiB / 4 | 1 | -0.0% | +0.1% | +0.2% | +0.7% |
| TCP 64 KiB / 4 | 2 | -0.5% | -1.9% | +0.4% | +0.2% |
| UDP 1200 B / 16 | 1 | -1.1% | -0.2% | -0.7% | -0.5% |
| UDP 1200 B / 16 | 2 | -0.2% | +0.6% | +0.9% | -0.4% |

Across all matched repetitions, the geometric mean changes are:

| Backend | Raw control | Ordinary pool | Legacy multishot | Worker-provided pool |
| --- | ---: | ---: | ---: | ---: |
| Tokio | -0.28% | -0.30% | +0.56% | — |
| io_uring | -0.35% | -0.39% | -0.01% | +0.62% |

These aggregates do not establish equivalent performance. The short-run one-worker tiny-TCP medians are −3.9% for io_uring legacy multishot and −2.4% for the Tokio ordinary pool; a longer focused comparison follows below.

### Longer tiny-TCP check

The further [168 io_uring samples](results/message-views-focused.jsonl) and [126 Tokio samples](results/message-views-focused-tokio.jsonl) reuse the same executables, locking, affinity, and randomized pairing. They isolate one-worker TCP with 64-byte messages and window one, using 21 matched repetitions and 15,000 timed batches per sample.

| Backend | Receive path | Median change | Repeat range |
| --- | --- | ---: | ---: |
| Tokio | raw | -0.10% | -2.12% to +3.75% |
| Tokio | pool | +0.67% | -2.46% to +3.62% |
| Tokio | multi | -0.47% | -4.46% to +3.72% |
| io_uring | raw | -1.13% | -4.92% to +1.98% |
| io_uring | pool | -0.69% | -3.50% to +4.75% |
| io_uring | multi | -0.69% | -3.52% to +5.09% |
| io_uring | provided | +0.95% | -6.23% to +3.87% |

The larger short-run slowdowns did not persist in the longer check: io_uring legacy multishot changes by −0.7%, while Tokio’s ordinary pool changes by +0.7%. The raw controls change by −1.1% and −0.1% respectively. These measurements support the accessor design without demonstrating a universal speedup or proving that access and conversion are free. They do not establish that provided buffers outperform ordinary monitor receives; that is a separate strategy comparison.

## Reproduce

Build both revisions separately:

```sh
cargo build --release --locked -p core-scheduler-buffer-benchmark --bin sockets
```

The runner refuses to overwrite an existing output file and records binary hashes:

```sh
flock /tmp/core-scheduler-perf.lock python3 benchmarks/buffers/message_views.py \
  --before /path/to/baseline/sockets --after /path/to/candidate/sockets \
  --output /tmp/message-views.jsonl
```

For the longer check, select one backend at a time with a fresh output path and `--backends iouring` or `--backends tokio`, plus `--workers 1 --workloads tcp-small --repeats 21 --iterations 15000`.
