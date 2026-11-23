# Watched cancellations in the echo benchmark

Screening evidence, gathered 24 November 2026 on [hana](https://github.com/base-rs/base-benchmark/blob/main/hosts/hana.md), for [watched cancellations](../../design/cancellation-watch.md): what a real cancellation costs each echo operation, plain and watched, against `Cancellation::never()`, without HTTP.

## Result

A watched cancellation costs what `never` does: throughput and server CPU per request within noise of it, at one connection and at 32. A plain one costs about 10% more user CPU per request, which lowers throughput by 2% at 32 connections, lower in all 7 repetitions.

| Connections | Cancellation | Median requests/s | Range | Server CPU µs/request | Server user µs/request |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | `never` | 77,997 | 77,499–78,629 | 7.53 | 1.90 |
| 1 | plain | 77,514 | 76,426–78,422 | 7.72 | 2.06 |
| 1 | watched | 78,048 | 77,489–80,427 | 7.55 | 1.91 |
| 32 | `never` | 241,332 | 236,361–247,511 | 3.98 | 0.98 |
| 32 | plain | 236,471 | 230,090–239,721 | 4.09 | 1.04 |
| 32 | watched | 241,336 | 235,772–247,323 | 3.97 | 0.95 |

Paired by repetition against `never`:

| Connections | Cancellation | Throughput | Lower in | Server user CPU per request | Higher in |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | plain | −0.8% | 5 of 7 | +10.8% | 6 of 7 |
| 1 | watched | +0.5% | 3 of 7 | +0.4% | 4 of 7 |
| 32 | plain | −2.0% | 7 of 7 | +9.3% | 7 of 7 |
| 32 | watched | +0.0% | 3 of 7 | −0.4% | 3 of 7 |

## Method

The `native-monitor` backend, Native readiness with borrowed reads and writes, served with one worker on CPU 3, against the benchmark's client with two workers on CPUs 1 and 2, for 64-byte messages in a closed loop: 7 interleaved repetitions of 3 seconds after 1 second of warmup. A `+plain` backend suffix gives each connection a child of one server-wide cancellation, and `+watched` that child's watch, as an HTTP server gives each connection; without a suffix, operations are given `never`. The run used `base-scheduler` at `a99df63`, `base-scheduler` 0.6.4 with the suffixes added, with Rust 1.99.1 on Linux 7.2.14; `run.py` recorded the clone as dirty because of an empty untracked file that the measuring command's lock redirection created in it, not a source change. [samples.json](results/watched-cancellation/samples.json) keeps every repetition and the run's metadata.

```sh
python3 benchmarks/echo/run.py --workers 1 --connections 1 32 --bytes 64 \
  --backends native-monitor native-monitor+plain native-monitor+watched \
  --repeats 7 --seconds 3 --warmup 1 --client-workers 2 \
  --server-cpus 3 --client-cpus 1,2 --output echo-watch.json
```
