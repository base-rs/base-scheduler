# Tokio admission-only closure performance

The Tokio adapter now checks scheduler admission when a socket or clock operation first runs. Accepted operations retain only the explicit caller cancellation wait. `close()` rejects new work without implicitly cancelling accepted socket or timer waits; applications cancel and await them explicitly. `shutdown()` drains adapter-owned tasks, native work, and cleanup. Socket and clock futures owned by external tasks are not separately tracked.

This removes the per-operation shutdown listener. With `Cancellation::never()`, socket and clock operations register no cancellation listener. Tokio readiness registration, syscall attempts, and the owned/borrowed interfaces are unchanged. This comparison measures that isolated change, including its admission flag and removal of listener state; it does not isolate individual instruction costs or trait dispatch.

## Repeated throughput

Median complete round trips per second across five repetitions on Aiko (Ryzen 9 9950X3D, Linux 7.2.6, Rust 1.99.0). The [raw report](results/tokio-admission/aiko.json) retains all 180 uninstrumented runs. Both executable builds include a pure Tokio control.

| Workers | Connections | Bytes | Tokio before | Tokio after | Owned before | Owned after | Borrowed before | Borrowed after |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 64 | 268,454 | 271,089 | 253,237 | 265,411 | 248,975 | 264,541 |
| 1 | 1 | 4096 | 234,291 | 232,406 | 230,513 | 233,550 | 231,299 | 233,434 |
| 1 | 32 | 64 | 699,627 | 694,179 | 577,175 | 598,472 | 578,182 | 593,454 |
| 1 | 32 | 4096 | 620,594 | 615,585 | 523,960 | 536,645 | 525,238 | 541,329 |
| 4 | 32 | 64 | 1,735,176 | 1,740,703 | 1,551,012 | 1,619,792 | 1,547,013 | 1,608,666 |
| 4 | 32 | 4096 | 1,610,404 | 1,606,619 | 1,438,232 | 1,499,199 | 1,453,418 | 1,488,361 |

Changes in median throughput and the resulting fraction of the same-build pure Tokio control:

| Workers | Connections | Bytes | Tokio change | Owned change | Borrowed change | Owned / Tokio after | Borrowed / Tokio after |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 64 | +1.0% | +4.8% | +6.3% | 97.9% | 97.6% |
| 1 | 1 | 4096 | -0.8% | +1.3% | +0.9% | 100.5% | 100.4% |
| 1 | 32 | 64 | -0.8% | +3.7% | +2.6% | 86.2% | 85.5% |
| 1 | 32 | 4096 | -0.8% | +2.4% | +3.1% | 87.2% | 87.9% |
| 4 | 32 | 64 | +0.3% | +4.4% | +4.0% | 93.1% | 92.4% |
| 4 | 32 | 4096 | -0.2% | +4.2% | +2.4% | 93.3% | 92.6% |

With 32 connections, removing the listener improves adapter median throughput by 2.4–4.4%. Every after-change adapter sample in those four cases exceeds every corresponding before-change sample. Pure Tokio control medians change by less than 1% across all six workloads. The improvement is useful but does not establish parity: with 32 connections the adapters reach 85.5–93.3% of the after-build pure Tokio control, leaving a 6.7–14.5% throughput gap.

For one connection and 64-byte messages, owned and borrowed throughput improves 4.8% and 6.3%, reaching roughly 98% of pure Tokio. For one connection and 4-KiB messages, all implementations have overlapping ranges; the adapters' approximately 1% gain and small lead over the after-build control do not establish a meaningful advantage. Neither the owned nor borrowed interface is consistently faster across the matrix.

These are ratios of medians, not confidence intervals or estimates of server-only capacity. The closed-loop client, kernel loopback networking, and scheduling all contribute. Earlier sampled CPU percentages for listener functions cannot be read as a prediction of wall-clock throughput gains.

## Variation and latency

Minimum–maximum throughput across the five repetitions:

| Workers | Connections | Bytes | Tokio before | Tokio after | Owned before | Owned after | Borrowed before | Borrowed after |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 64 | 267,984–270,191 | 266,205–271,854 | 247,686–256,139 | 263,592–266,789 | 243,783–254,782 | 260,802–268,147 |
| 1 | 1 | 4096 | 233,141–238,098 | 230,184–240,676 | 227,743–234,790 | 230,199–236,494 | 227,540–236,632 | 230,983–235,452 |
| 1 | 32 | 64 | 696,005–708,648 | 690,888–700,744 | 575,863–579,874 | 594,504–600,745 | 574,005–582,305 | 589,063–595,499 |
| 1 | 32 | 4096 | 618,208–622,192 | 611,621–620,423 | 520,422–524,304 | 532,793–540,761 | 524,178–526,738 | 532,412–542,191 |
| 4 | 32 | 64 | 1,715,157–1,747,946 | 1,702,267–1,759,425 | 1,544,806–1,555,999 | 1,611,029–1,637,437 | 1,525,442–1,563,946 | 1,596,769–1,616,316 |
| 4 | 32 | 4096 | 1,588,647–1,621,460 | 1,595,908–1,607,560 | 1,421,717–1,453,219 | 1,489,684–1,516,654 | 1,444,816–1,457,925 | 1,482,097–1,493,319 |

Median per-run p50 / p99 round-trip latency in microseconds; these are not pooled percentiles:

| Workers | Connections | Bytes | Tokio before | Tokio after | Owned before | Owned after | Borrowed before | Borrowed after |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 64 | 3.59 / 4.37 | 3.58 / 4.36 | 3.63 / 5.71 | 3.62 / 5.30 | 3.66 / 5.69 | 3.63 / 4.81 |
| 1 | 1 | 4096 | 3.92 / 5.42 | 4.05 / 5.41 | 4.01 / 5.51 | 3.95 / 5.48 | 3.97 / 5.50 | 3.96 / 5.43 |
| 1 | 32 | 64 | 46.69 / 53.50 | 47.07 / 53.92 | 56.70 / 62.56 | 54.66 / 60.93 | 56.61 / 62.53 | 55.10 / 61.41 |
| 1 | 32 | 4096 | 52.70 / 60.09 | 53.12 / 60.48 | 62.49 / 69.25 | 60.96 / 67.78 | 62.34 / 68.99 | 60.48 / 67.33 |
| 4 | 32 | 64 | 17.54 / 52.90 | 17.50 / 52.64 | 19.63 / 56.16 | 18.70 / 53.70 | 19.47 / 55.94 | 18.75 / 53.76 |
| 4 | 32 | 4096 | 18.93 / 60.16 | 18.98 / 60.41 | 21.02 / 61.38 | 20.02 / 60.73 | 20.78 / 61.12 | 20.11 / 60.77 |

Median per-run p99 decreases in all twelve adapter comparisons. With 32 connections, mean RTT also decreases in each comparison. The concurrent adapter paths still have higher p50 and p99 medians than their same-build pure Tokio controls. The raw maxima and sample variation remain relevant; five short runs do not establish tail-latency bounds.

Raw records also retain mean and maximum RTT and child-process CPU accounting. CPU totals cover the whole process lifecycle, including warmup and shutdown, while request counts cover only the measurement window; they are not exact per-request CPU costs.

## Method and provenance

The [comparison runner](results/tokio-admission/run.py) reuses the existing profiling runner's uninstrumented sample and process CPU accounting. Each repetition shuffles all six workload cases and six build/backend combinations with fixed seed `20261024`. Each sample has one second of warmup and three seconds of measurement, and uses separate server/client processes. All runs execute sequentially, with no concurrent build, test, or profiler. Server workers use CPU 0 or CPUs 0–3; the fixed four-worker client uses CPUs 4–7, on separate physical cores sharing the same last-level cache.

- Before executable: `a0209b7211a2ce62306be1659bf766f0e4bc87a3`. Its Tokio crate, benchmark Rust source, and lockfile are identical to pre-change HEAD `4c9e1dd60604153748040eccd559460c21c13fb3`.
- After executable: `af945bfeb9bc7bdaf906a6122b18fb289bf91d03`.
- Fixed client: `5736301fded46c4e5f11d5387950ae2ee0807b94`.

Both servers use `cargo build --release --locked -p core-scheduler-echo-benchmark` with `CARGO_PROFILE_RELEASE_DEBUG=1` and separate target directories. This is optimization level 3, with default release code generation and LTO settings, and no `target-cpu=native`. Debug symbols are enabled for both builds. Neither server nor client uses coverage instrumentation. All Core echo operations pass `Cancellation::never()` and reuse their per-connection buffer. The benchmark's [methodology](readme.md) documents validation, partial transfers, measurement boundaries, and loopback limitations.

Executable SHA-256 hashes:

| Executable | SHA-256 |
| --- | --- |
| before | `083f86684af85171f97ede2eae4cdcac09adb9c276988ab01f85e32c942e19df` |
| after | `ae485daa8ad173c43fa18b6eaa3c3f9342fbad19d30258ae8a365d20ba7b0fc1` |
| client | `db6c914ab75011477ec6232e681046b1e75532cbaa1c81663bac984591675cf3` |

Reproduce with the retained executables:

```sh
python3 benchmarks/echo/results/tokio-admission/run.py \
  --before /tmp/core-ring-readiness-profile/release/core-scheduler-echo-benchmark \
  --after /tmp/core-tokio-admission/release/core-scheduler-echo-benchmark \
  --client /tmp/core-scheduler-profile-baseline/release/core-scheduler-echo-benchmark \
  --before-source a0209b7211a2ce62306be1659bf766f0e4bc87a3 \
  --after-source af945bfeb9bc7bdaf906a6122b18fb289bf91d03 \
  --client-source 5736301fded46c4e5f11d5387950ae2ee0807b94 \
  --output /tmp/core-tokio-admission-results
```

## Behavioral verification

Regressions verify successful completion after close/shutdown, explicit cancellation after closure with original buffer retention, rejection of previously constructed but unpolled operations, and task-owned I/O keeping shutdown pending until cancellation. Paused-clock tests cover accepted timer completion and cancellation after closure. Targeted Bake coverage passes all 2,557 measured source regions on macOS and all 2,703 on Linux; the full Linux/macOS/Windows CI matrix also passes coverage, formatting, Clippy, documentation, and Rust 1.85 compatibility for the semantic change.
