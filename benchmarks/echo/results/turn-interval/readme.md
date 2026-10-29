# Readiness worker turn spacing

Between ready batches, a busy Native worker makes a zero-timeout driver turn. For the epoll and kqueue readiness worker, each such turn is an `epoll_pwait`/`kevent` call. On one worker, a task that repeatedly refreshes a timer and yields takes 760 ns per iteration, against Tokio's 359 ns in the same run ([timers-all-t0.jsonl](timers-all-t0.jsonl)), because every one-task batch pays that call. Three ways to space the turns were measured against real HTTP servers. The two that were timed remove the yield-loop cost. All three were rejected: none improves the HTTP path, and each regresses it in some configuration. The batch structure on main is unchanged.

All candidates start from main `aaf337eab1d0fc99e6177a61649dfc94fe6b30f1`. The patches are retained here because their branches are not published:

| Variant | Patch | Change |
| --- | --- | --- |
| A | — | main |
| B | [interval.patch](interval.patch) | `SelectorWorker::nonblocking_interval`; the readiness worker turns at the first batch boundary after 61 task polls |
| C | [fifo.patch](fifo.patch), on B | One FIFO per worker instead of two alternating queues. A batch is the queue length counted before polling it, so self-wakes queue behind its members |
| C+cap | C, with `BASE_NATIVE_BATCH_CAP=1` | C, with readiness batches capped at 61 polls. The cap applies only when the interval exceeds one, so on io\_uring C+cap is the same configuration as C |
| E | [adaptive.patch](adaptive.patch) | Skip zero-timeout readiness turns after a turn that found no events, backing off up to 60 skips; any productive or waiting turn resets it |

[echo-native-backend.patch](echo-native-backend.patch) adds the `native`, `native-poll` and `native-monitor` echo backends used for the earlier echo profiling. It is retained for reproduction only.

## Environment and method

Hana, an Intel Xeon E3-1240 v6 with 4 cores and 8 threads, running Linux 7.2 with the powersave governor; [environment.txt](environment.txt) records the toolchain. The HTTP server is the base-http hello benchmark at `519d476`, built against each scheduler variant through `[patch.crates-io]` ([hbuild.sh](hbuild.sh), which also enables Tokio's `rt-multi-thread` feature in the hello `Cargo.toml`). [hello-patch.py](hello-patch.py), applied to the hello `src/main.rs`, adds hana-only backends without changing the existing ones:

- `WORKERS` overrides the Native and io_uring worker count;
- `base-native-tasks` and `base-iouring-tasks` spawn one task per HTTP connection, instead of polling every connection in one `FuturesUnordered` supervisor as `base_http_server::serve` does;
- `hyper-spawn-tokio` runs Hyper on a multi-threaded Tokio runtime with one task per connection.

[hbench.sh](hbench.sh) pins the server and the wrk client: CPU 0 and CPUs 1–3,5–7 with one worker, CPUs 0–1 and CPUs 2,3,6,7 with two. Each sample warms up for one second, then counts user instructions, user cycles and task clock with `perf stat` over a 3.2-second window around a three-second wrk run. Instructions per request usually repeat to within 0.2–0.5%; requests per second usually vary about ±4%, with occasional samples about 10% low. [hcohort.sh](hcohort.sh) interleaves A, B, C and C+cap over three repeats ([http-cohort.jsonl](http-cohort.jsonl)); [ecohort.sh](ecohort.sh) interleaves A and E over five ([http-cohort-E.jsonl](http-cohort-E.jsonl)), after first running the Native timer suite. Timer results use `benchmarks/timers/run.py` with three repeats ([timers-native-t0.jsonl](timers-native-t0.jsonl) for A, `-t3` for B, `-tE` for E). C and C+cap were not timed separately. The scripts retain the hana paths they were run with.

These are closed-loop loopback measurements. They do not establish latency at a fixed offered rate.

## Timers

The ns per operation (wall / CPU) for Native in `reuse` mode:

| Workload | Workers / tasks | A | B | E |
| --- | --- | ---: | ---: | ---: |
| yield | 1 / 1 | 760 / 759 | 251 / 251 | 256 / 256 |
| yield | 4 / 64 | 131 / 520 | 126 / 501 | 126 / 500 |
| cycle | 4 / 64 | 502 / 1,235 | 508 / 960 | 506 / 1,100 |
| refresh | 1 / 1 | 124 / 123 | 125 / 125 | 126 / 126 |
| fixed | 1 / 1 | 109 / 109 | 109 / 109 | 110 / 110 |
| expire 1 ms | 1 / 1 | 1.07 ms / 26.2 µs | 1.07 ms / 28.3 µs | 1.08 ms / 30.4 µs |
| expire 1 ms | 4 / 128 | 8,642 / 3,983 | 8,675 / 4,395 | 8,735 / 4,642 |

The single-worker yield loop is the only material wall-clock change. CPU time moves both ways: the four-worker `cycle` workload falls by 22% under B and 11% under E, while timer expiry rises by 8–17%.

## HTTP

Medians as requests per second / user instructions per request:

| Workers | Server | Connections | A | B | C | C+cap |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | supervisor | 64 | 182.9k / 5,887 | 186.8k / 5,887 | 182.1k / 5,887 | 182.9k / 5,884 |
| 1 | supervisor | 256 | 176.8k / 5,876 | 173.7k / 5,875 | 174.0k / 5,876 | 173.7k / 5,872 |
| 1 | io_uring supervisor | 64 | 185.5k / 5,986 | 193.2k / 5,979 | 187.7k / 5,983 | 189.3k / 5,986 |
| 1 | io_uring supervisor | 256 | 178.7k / 5,995 | 179.6k / 5,991 | 179.0k / 5,996 | 177.2k / 5,995 |
| 1 | task per connection | 64 | 182.6k / 5,966 | 181.4k / 5,964 | 177.3k / 5,968 | 180.6k / 5,965 |
| 1 | task per connection | 256 | 180.0k / 5,998 | 167.9k / 5,998 | 177.4k / 5,992 | 173.4k / 5,974 |
| 2 | task per connection | 64 | 366.3k / 6,021 | 361.7k / 6,177 | 347.8k / 6,172 | 350.9k / 6,162 |
| 2 | task per connection | 256 | 383.5k / 5,994 | 365.9k / 6,014 | 376.7k / 6,010 | 369.0k / 6,002 |
| 2 | io_uring, task per connection | 64 | 365.9k / 6,152 | 371.4k / 6,152 | 374.7k / 6,169 | 374.8k / 6,159 |
| 2 | io_uring, task per connection | 256 | 366.2k / 6,206 | 366.6k / 6,216 | 369.3k / 6,213 | 376.6k / 6,212 |

- **The supervisor server's instructions are unchanged.** Likely it rarely runs consecutive batches, so there was little turn work to save; no diagnostic here measures that.
- **Two epoll workers regress under every interval variant.** At 64 connections, all nine B, C and C+cap samples are at least 6,159 instructions per request, about 150 more than all three A samples (6,021–6,022). At 256 connections the increase is only 8–20.
- **One worker with a task per connection at 256 connections** is slower under B in every sample (162.5–170.1k, against A's 179.0–183.5k).
- **The single FIFO (C) shows no gain over the alternating queues.** Instructions match B. On io\_uring, C is 0.2–2.4% above A in throughput, which is within noise.
- **One connection** is level in throughput. Cycles per request mostly rise (−1% to +13%), including on io\_uring where B does not change turns, so treat them as noisy at three repeats.

The second cohort, as requests per second / instructions per request / cycles per request:

| Workers | Server | Connections | A | E |
| --- | --- | ---: | ---: | ---: |
| 1 | supervisor | 64 | 186.4k / 5,884 / 4,028 | 178.2k / 5,885 / 4,414 |
| 1 | supervisor | 256 | 183.8k / 5,860 / 4,165 | 177.0k / 5,867 / 4,462 |
| 1 | task per connection | 64 | 184.4k / 5,963 / 4,205 | 178.9k / 5,963 / 4,419 |
| 1 | task per connection | 256 | 179.5k / 5,993 / 4,362 | 178.5k / 5,994 / 4,392 |
| 2 | task per connection | 64 | 365.8k / 6,018 / 4,568 | 357.5k / 6,022 / 4,847 |
| 2 | task per connection | 256 | 376.7k / 6,005 / 4,689 | 374.8k / 5,999 / 4,671 |

E removes B's instruction regression but costs cycles and throughput. With one worker and a task per connection at 64 connections, E is slower, with more cycles per request, in each of the five paired repeats. One-connection throughput is level.

For reference, Hyper on single-threaded Tokio served 176.6k and 161.6k requests per second at 64 and 256 connections, and Hyper with one task per connection on two Tokio workers served 330.4k and 360.0k.

## Decision

Reject all three. The yield-loop gain does not appear in HTTP serving, which either does not change or regresses by 1–7%. The alternating ready queues stay: the queue swap costs nothing measurable and already bounds a batch by membership. Revisit turn spacing only with a workload where consecutive self-woken batches dominate, and require no HTTP regression in instructions or throughput at one and two workers.
