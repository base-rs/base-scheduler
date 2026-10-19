# Borrowed socket performance

The echo benchmark compares both owned operations and borrowed `socket_recv_poll` / `socket_send_poll` operations against pure Tokio. The goal is to measure the fastest available socket path as well as the completion path. An operation that completes immediately can avoid registration and rescheduling, but the cost of waiting for the next readable socket still matters in this closed-loop workload.

## Implementations

All five cases use the same server binary and reuse one 64-KiB allocation per connection. The Core cases use `Cancellation::never()`, default send/receive flags, and a loop that finishes partial writes. Pure Tokio uses `TcpStream::read` / `write_all`.

- `tokio-adapter` and `tokio-adapter-poll` both use Tokio readiness. Borrowing changes the buffer interface without changing the underlying reactor.
- `iouring` submits owned buffers to per-worker rings and resumes tasks through completion channels.
- `iouring-poll` keeps those same native workers and ring drivers, but sends borrowed socket operations through the shared `async-io` readiness fallback. No payload pointers are submitted to the rings. A ready syscall completes inline; a pending operation relies on the separate readiness reactor thread to wake a native worker. All server threads inherit the server's CPU affinity, including that reactor thread.

This compares usable implementations, not ownership in isolation. The owned interface also permits immediate nonblocking syscall attempts; it does not require every operation to allocate a request or pass through a completion queue.

## Repeated results

Median complete round trips per second across five repetitions, on Aiko (Ryzen 9 9950X3D, Linux 7.2.6, Rust 1.99.0). The [one-worker samples](results/poll/aiko-one-worker.json) and [four-worker samples](results/poll/aiko-four-workers.json) preserve all 150 runs, machine details, and executable hashes.

| Workers | Connections | Bytes | Pure Tokio | Tokio owned | Tokio `_poll` | Native owned | Native `_poll` |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 64 | 273,484 | 249,024 | 241,932 | 233,953 | 159,053 |
| 1 | 1 | 4096 | 236,414 | 233,111 | 231,675 | 187,573 | 140,765 |
| 1 | 32 | 64 | 692,975 | 579,466 | 580,043 | 519,272 | 405,010 |
| 1 | 32 | 4096 | 609,320 | 518,246 | 520,933 | 466,382 | 360,282 |
| 4 | 32 | 64 | 1,716,479 | 1,533,301 | 1,563,911 | 1,476,063 | 1,346,355 |
| 4 | 32 | 4096 | 1,602,593 | 1,426,916 | 1,439,569 | 1,354,579 | 1,168,083 |

Native `_poll` is slower in throughput than owned io_uring in all six cases: about 22–32% with one worker and 9–14% with four workers. In every case, even the fastest native `_poll` run is below the slowest owned io_uring run. The initial expectation that bypassing completion requests would automatically improve this workload was wrong for the current implementation. Its ready syscall path is short, but waiting goes through an independently driven readiness reactor and wakes a worker that is driving a different selector. The comparison identifies that integration as an optimization target; it does not isolate how much of the difference each mechanism causes.

The Tokio adapter changes much less: its borrowed mode ranges from about 3% slower to 2% faster than its owned mode in median throughput, with overlapping sample ranges in most cases. Both already use the same readiness integration. Pure Tokio remains faster overall in these measurements.

Median of each run's p50 / p99 round-trip latency, in microseconds; these are not pooled percentiles:

| Workers | Connections | Bytes | Pure Tokio | Tokio owned | Tokio `_poll` | Native owned | Native `_poll` |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 64 | 3.53 / 4.32 | 3.64 / 5.78 | 3.67 / 5.82 | 4.07 / 5.84 | 6.00 / 8.04 |
| 1 | 1 | 4096 | 3.88 / 5.45 | 3.97 / 5.46 | 3.99 / 5.53 | 5.05 / 8.04 | 7.13 / 8.99 |
| 1 | 32 | 64 | 47.17 / 53.98 | 56.48 / 62.43 | 56.45 / 62.34 | 61.05 / 69.31 | 85.18 / 120.25 |
| 1 | 32 | 4096 | 53.70 / 61.02 | 63.20 / 70.02 | 62.88 / 69.63 | 68.03 / 76.99 | 94.21 / 133.50 |
| 4 | 32 | 64 | 17.66 / 52.83 | 19.77 / 56.35 | 19.50 / 55.36 | 20.24 / 56.67 | 18.45 / 53.95 |
| 4 | 32 | 4096 | 18.98 / 60.45 | 21.26 / 61.70 | 21.05 / 61.31 | 22.09 / 65.98 | 19.63 / 63.36 |

The throughput result does not imply uniformly worse latency. With four workers, native `_poll` has lower p50 and p99 medians than owned io_uring for both payload sizes, despite lower throughput. However, median per-run mean latency rises from 21.63 to 23.72 µs for 64-byte messages and from 23.54 to 27.31 µs for 4-KiB messages, consistent with the throughput decrease. Lower p50 and p99 do not imply a lower mean; delays beyond p99 still matter. Raw records include mean latency and the observed maximum, while the table reports unpooled per-run percentiles.

Minimum–maximum throughput across the five repetitions:

| Workers | Connections | Bytes | Pure Tokio | Tokio owned | Tokio `_poll` | Native owned | Native `_poll` |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 64 | 268,734–276,649 | 242,587–257,421 | 235,860–262,764 | 230,573–237,497 | 157,481–168,368 |
| 1 | 1 | 4096 | 230,691–241,185 | 230,930–241,711 | 230,022–234,240 | 171,900–190,515 | 135,912–144,445 |
| 1 | 32 | 64 | 684,064–696,664 | 576,181–582,095 | 576,997–582,222 | 515,705–522,245 | 401,146–406,221 |
| 1 | 32 | 4096 | 608,283–617,792 | 517,277–527,307 | 518,624–523,704 | 464,432–469,762 | 357,067–361,559 |
| 4 | 32 | 64 | 1,694,963–1,760,720 | 1,517,872–1,550,855 | 1,541,922–1,566,009 | 1,441,425–1,494,835 | 1,325,859–1,363,481 |
| 4 | 32 | 4096 | 1,600,913–1,614,560 | 1,419,514–1,433,006 | 1,427,357–1,445,706 | 1,341,880–1,365,362 | 1,142,034–1,193,812 |

These short repeated runs establish the direction and approximate magnitude for this workload, not confidence intervals or a general ranking of readiness versus completion I/O.

## Next optimization targets

The native borrowed path currently uses a readiness service designed as a portable fallback. A worker-integrated readiness path would attempt the syscall directly and, on `WouldBlock`, wait through that worker's selector. Possible mechanisms include io_uring `PollAdd` or readiness registrations in an integrated poller. Readiness requests must retain the socket safely and handle cancellation and task migration, but must not retain pointers to the borrowed payload. Removing the separate reactor handoff is a concrete experiment; this comparison alone cannot quantify its benefit.

The owned completion path has independent opportunities: immediate syscall attempts when appropriate, reusable request/completion state, and less worker-wakeup coordination. Those should be measured separately. Borrowing alone does not guarantee a faster implementation, and the current fallback result does not establish a performance limit for either interface.

## Method

Source commit `7cd6af8725fedebc2dffd52473380d51d7d7fc5e`, built with `cargo build --release --locked` and the default release profile. All five server modes come from this same build. The client is the unchanged original baseline executable from `5736301fded46c4e5f11d5387950ae2ee0807b94`; the raw files record both executable hashes. These measurements are a fresh comparison, rather than ratios against the earlier builds with debug symbols enabled.

Each case runs five times with one second of warmup and three seconds of measurement. Within each repetition the case and backend order are shuffled with the runner's fixed seed. Server workers use CPU 0 or CPUs 0–3; the client uses four workers on CPUs 4–7. The one-worker and four-worker matrices run sequentially, with no builds or profilers running alongside timed measurements. The [benchmark methodology](readme.md) describes payload validation, measurement windows, and the limitations of closed-loop TCP loopback tests.

Reproduce with the fixed client executable retained from the original benchmark:

```sh
python3 benchmarks/echo/run.py --workers 1 --connections 1 32 --repeats 5 --server-cpus 0-3 --client-cpus 4-7 --client-binary /tmp/core-scheduler-profile-baseline/release/core-scheduler-echo-benchmark --output /tmp/core-echo-poll-one-worker.json
python3 benchmarks/echo/run.py --workers 4 --connections 32 --repeats 5 --server-cpus 0-3 --client-cpus 4-7 --client-binary /tmp/core-scheduler-profile-baseline/release/core-scheduler-echo-benchmark --output /tmp/core-echo-poll-four-workers.json
```

## Validation

Benchmark tests cover all available modes, partial writes, zero-progress writes, buffer reuse, and payload verification. The changed benchmark package passes 491/491 measured source regions on Linux and 461/461 on macOS; formatting and Clippy pass on both platforms. No production scheduler implementation changes are included in this comparison.
