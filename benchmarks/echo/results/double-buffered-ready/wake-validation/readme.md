# Worker wakeup validation

Suppressing a Native worker's redundant self-interrupt recovers the observed serial slowdown while retaining the alternating queues' concurrent gains in these echo measurements. Two new timing blocks put fixed owned serial throughput within 0.5% of the original loop; four-worker throughput remains 1.4–3.2% higher than original. All platform coverage gates, repeated worker tests, and idle-parking checks pass. The evidence supports the fix without proving statistical equivalence or general application performance. Earlier diagnosis and prototype measurements remain separate.

## Implementation and progress

`WorkerState::wake` keeps its sequentially consistent sleeping-flag exchange. If that exchange finds a sleeping worker, it clears the flag and interrupts only a different thread. A wake from the worker already processing driver callbacks needs no `Thread::unpark` or driver notification: the driver must return so the executor can poll its published work. This avoids leaving a notification permit for an unnecessary empty driver turn.

The `SelectorWorker::turn` contract explicitly requires returning after **any task wake**, including completion publication, cancellation, and owner destruction. It must not wait for its own executor to interrupt it. Every processed io_uring command or CQE already marks its turn as productive, satisfying this requirement. Remote publication, retained unpark tokens, driver interrupts, and stealable ready queues remain intact.

| Variant | Source | Binary SHA-256 |
| --- | --- | --- |
| Original single-queue loop | `ddc994d2f1f3f10df53e46241a730d38a7c37531` | `9775b17d5b32d630ac718e0dd5a39fd5fb29e157d733b5c69cd4355da5516383` |
| Uncorrected alternating queues | `d1306dfc7ccde4502d4e966cc4e070bc32d2e101` | `1d50c1178fdb6e7902b3c50dea5c541585d8c670e387afcda956b387fff1f327` |
| Fixed alternating queues | `9386f7c4f6cb0dccd7e528512476d2cf963f7a88` | `1e58573bd54ab7e4339101298e31846f9ec26f9951080fe441f51e63c0d99e1e` |

Commit `2df2721781a2e1bf9d27ca21914e56692308f49f` broadens the comments and design contract from completion publication to all task wakes. It changes no executable code or tests after the measured and tested source. The [patch](fix.patch) contains implementation, regression tests, and the final contract relative to the uncorrected source. [Build metadata](build.json) and the [release build log](build.log) record the fixed binary's provenance.

## Correctness checks

The [check record](checks.json) and [successful CI run](https://github.com/core-rs/core-scheduler/actions/runs/39465549745) cover the implementation source above:

- Full workspace coverage with all targets/features: Linux **18,447/18,447**, macOS **15,859/15,859**, Windows **15,366/15,366** measured source regions. Linux excludes one invariant-only unreachable region; macOS and Windows exclude none.
- Formatting, workspace Clippy with warnings denied, documentation tests, and three additional timer feature configurations pass on all three CI platforms. Rust 1.85 compatibility passes.
- Local macOS and Aiko Linux full workspace coverage independently pass with the same counts. Linux includes the real io_uring tests and conformance suites. The local Linux wrapper filters Arch compiler source paths, not project sources.
- The [stress run](stress.json) repeats worker and controlled-driver tests 100 times each: **2,100 test executions passed**.
- An independent read-only review finds no remaining correctness issue in sleeping-state publication, remote wake races, driver progress, queue discovery, retirement, or shutdown. It requested the broader task-wake contract, then confirmed that clarification on `2df2721`.

New tests check local sleeping-state clearing without a driver interrupt, a remote wake immediately before parking with a retained unpark token and exactly one driver interrupt, and 32 controlled idle-completion batches that resume tasks without polling them inline or interrupting the publishing worker. Shutdown completes afterward. Existing tests cover task self-wakes, both stealable generations, driver failure/retirement, cancellation, and draining. A dedicated non-completion callback/drop wake regression would be additional hardening; the review checked those shipped io_uring paths, but the new controlled test specifically publishes completions.

## Full workload matrix

The completed [120 samples](matrix/aiko.json) and [summary](matrix/summary.json) retain the concurrent gains while recovering most of the uncorrected serial loss. With four workers and 128 connections, fixed owned throughput exceeds the original loop by **1.44–2.67%**; borrowed `_poll` exceeds it by **2.78–3.25%**. Concurrent fixed-versus-uncorrected changes range from **−0.35% to +1.00%**, providing no indication of a material concurrent throughput regression from suppressing self-interrupts in these samples. The fixed-versus-original concurrent p99 medians improve in all eight Native cells.

The one-connection owned median rises **3.01%** versus uncorrected and is **0.44%** below original, with broad overlapping ranges. The unchanged Tokio control's original/fixed median changes by **−1.49%** for the same workload. The separately planned serial confirmation below checks this variable case without pooling it into the matrix.

| Workers/connections/bytes | Backend | Original req/s [range] | Uncorrected req/s [range] | Fixed req/s [range] | Fixed/original | Fixed/uncorrected | Median per-run p99 µs, original → uncorrected → fixed |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1/1/64 | iouring | 240,995 [236,112–246,870] | 232,921 [228,605–233,416] | 239,937 [233,304–250,190] | -0.44% | +3.01% | 5.803 → 6.023 → 5.971 |
| 1/1/64 | iouring-poll | 236,018 [235,162–239,325] | 237,139 [228,071–238,306] | 239,691 [235,164–246,467] | +1.56% | +1.08% | 6.483 → 5.751 → 5.863 |
| 1/1/64 | tokio | 274,217 [271,405–275,359] | — | 270,138 [270,006–276,159] | -1.49% | — | 4.363 → — → 4.351 |
| 1/32/64 | iouring | 633,848 [632,273–634,464] | 637,050 [636,411–639,326] | 635,267 [634,963–636,721] | +0.22% | -0.28% | 57.439 → 54.111 → 54.239 |
| 1/32/64 | iouring-poll | 571,139 [565,435–573,161] | 572,505 [572,133–576,399] | 573,198 [571,547–574,152] | +0.36% | +0.12% | 62.079 → 60.127 → 60.159 |
| 1/32/64 | tokio | 701,014 [700,972–701,665] | — | 702,591 [700,732–704,930] | +0.22% | — | 53.439 → — → 53.311 |
| 1/32/4096 | iouring | 564,776 [564,670–566,751] | 567,036 [563,022–570,823] | 565,399 [563,688–571,123] | +0.11% | -0.29% | 64.799 → 61.055 → 61.183 |
| 1/32/4096 | iouring-poll | 511,371 [509,480–513,095] | 513,559 [512,619–518,466] | 518,683 [515,345–519,971] | +1.43% | +1.00% | 68.863 → 67.199 → 66.431 |
| 1/32/4096 | tokio | 619,786 [611,069–624,449] | — | 611,301 [609,039–621,442] | -1.37% | — | 60.159 → — → 60.927 |
| 4/128/64 | iouring | 2,061,864 [2,061,529–2,066,259] | 2,096,263 [2,086,958–2,105,673] | 2,091,467 [2,075,065–2,114,750] | +1.44% | -0.23% | 112.767 → 107.391 → 107.775 |
| 4/128/64 | iouring-poll | 2,069,334 [2,067,873–2,070,226] | 2,134,207 [2,122,640–2,134,243] | 2,126,824 [2,121,842–2,128,986] | +2.78% | -0.35% | 117.631 → 105.151 → 105.471 |
| 4/128/64 | tokio | 2,121,183 [2,106,394–2,129,678] | — | 2,123,000 [2,110,793–2,132,481] | +0.09% | — | 102.655 → — → 101.759 |
| 4/128/4096 | iouring | 1,811,555 [1,811,305–1,814,326] | 1,860,673 [1,855,538–1,862,743] | 1,859,858 [1,851,941–1,860,050] | +2.67% | -0.04% | 128.063 → 119.423 → 119.167 |
| 4/128/4096 | iouring-poll | 1,831,867 [1,831,159–1,832,935] | 1,888,207 [1,882,708–1,890,777] | 1,891,343 [1,879,720–1,891,538] | +3.25% | +0.17% | 130.559 → 117.887 → 117.055 |
| 4/128/4096 | tokio | 1,864,614 [1,859,715–1,868,460] | — | 1,872,103 [1,869,782–1,886,987] | +0.40% | — | 117.567 → — → 116.927 |

The one-worker concurrent gains are small and should be interpreted alongside controls that move from −1.37% to +0.22%. At four workers, fixed owned throughput is within 1.5% of the same fixed executable's Tokio control; borrowed throughput is within about 1.1%, slightly above it. Native remains materially slower with one worker. These measurements support retaining the four-worker batching gain, not claiming general superiority over Tokio.

## Separate serial confirmation and idle parking

The separately planned [25-sample confirmation](serial/aiko.json) and [summary](serial/summary.json) recover **2.28%** owned serial throughput versus the uncorrected loop. The fixed median is **0.47%** below the original; the unchanged Tokio control shifts **−1.02%** between those builds. Fixed owned median per-run p99 returns to **5.771 µs**, versus original **5.791 µs** and uncorrected **6.071 µs**.

| Workers/connections/bytes | Backend | Original req/s [range] | Uncorrected req/s [range] | Fixed req/s [range] | Fixed/original | Fixed/uncorrected | Median per-run p99 µs, original → uncorrected → fixed |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1/1/64 | iouring | 241,449 [240,153–245,763] | 234,953 [230,089–241,055] | 240,317 [233,690–247,359] | -0.47% | +2.28% | 5.791 → 6.071 → 5.771 |
| 1/1/64 | tokio | 272,143 [268,168–275,617] | — | 269,357 [269,209–272,167] | -1.02% | — | 4.331 → — → 4.343 |

Both new timing blocks put the fixed owned serial median within half a percent of original, versus a 2.3–3.0% recovery over uncorrected. The repeated material serial loss seen before the fix is no longer evident, but wide overlapping ranges and the moving Tokio control do not prove exact equivalence or a small serial advantage. The causal diagnosis and the two new comparisons together support the self-interrupt fix.

The [twelve idle probes](idle.json) cover original/fixed, one/four workers, and Tokio/owned/borrowed servers. Every server records **zero process CPU ticks** during its two-second observation after a half-second warmup. No idle spin is observed at that resolution. Graceful shutdown is covered by the correctness tests rather than these idle probes, which terminate their server processes.

## Measurement procedure and limits

The plan in [build.json](build.json) was recorded before measurement: a 120-sample matrix, a separate 25-sample owned serial confirmation, then twelve idle-parking checks. The two timing blocks are kept separate. The matrix has three randomized interleaved repetitions of five workloads and eight variant/backend cases. The serial block has five repetitions of original/uncorrected/fixed owned I/O and both unchanged Tokio controls.

All binaries use `cargo build --release --locked -p core-scheduler-echo-benchmark`, the default optimized release profile (`opt-level=3`, no LTO/CPU overrides). The original and uncorrected binaries are the frozen executables from the earlier experiment. The fixed client is unchanged: source `5736301fded46c4e5f11d5387950ae2ee0807b94`, SHA-256 `db6c914ab75011477ec6232e681046b1e75532cbaa1c81663bac984591675cf3`.

The [runner](../run.py), [sample helper](../sample-runner.py), and [idle probe](../idle.py) are unchanged. Run the runner with `--output DIR --build build.json --client CLIENT`, the source table's binaries as `--case LABEL,BACKEND,BINARY`, and each of the five `--workload WORKERS,CONNECTIONS,BYTES` values. The matrix uses three repetitions, original/fixed Tokio controls, and all three variants of `iouring` and `iouring-poll`. The serial confirmation uses five repetitions, only `1,1,64`, the three owned variants, and both Tokio controls. Idle `before` means original; `after` means fixed.

Each timing sample warms for one second and measures for three seconds, using persistent validated IPv4 loopback TCP echo, `TCP_NODELAY`, reusable buffers, and one outstanding request per connection. Server affinity is CPU 0 or 0–3, client affinity CPUs 4–7, with powersave governors. All Aiko builds, tests, profiling, and timings in this work serialize on `/tmp/core-scheduler-perf.lock`; unrelated host activity is not excluded. Process CPU totals cover complete process lifetimes. RTT includes client scheduling.

The [summary script](summarize.py) rejects unfinished, missing, or duplicated samples, reports observed throughput ranges, and takes the median of each run's p99 rather than inventing a pooled percentile. Run `python3 summarize.py matrix/aiko.json --json` and similarly for `serial/aiko.json` to reproduce the summaries.

Short loopback samples and unchanged Tokio controls cannot prove statistical equivalence or a general advantage over Tokio. These runs do not measure the Native readiness selector's performance, physical networks, very large ready batches, fairness under continuous task spawning, or general applications. The loop still has no maximum batch size or preemption of an individual future poll.
