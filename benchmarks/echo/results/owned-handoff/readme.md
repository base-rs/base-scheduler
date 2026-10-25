# Owned receive bookkeeping

These experiments follow the lazy-quiescence change in `f5dbd04`. The selected runtime source is `83433057fc469e076dcd7e7f42a61b8147a91ced`. The public interface, owned-buffer safety, cooperative cancellation, abandonment, panic containment, migration, and shutdown behavior remain unchanged.

## Confirmation

Seven repeats per variant and case, shuffled with seed 20261025. Rates are medians of validated echoes per second; ranges include every sample. P99 is the median of each sample's reported P99, not a combined histogram.

| Workers / connections / bytes | Merged baseline | Candidate | Tokio | Candidate change | Candidate vs Tokio |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 / 1 / 64 | 260,162 | 269,854 | 274,636 | +3.73% | −1.74% |
| 1 / 32 / 64 | 648,318 | 662,753 | 712,051 | +2.23% | −6.93% |

The serial ranges are 254,475–269,519 for baseline, 265,390–272,185 for candidate, and 272,155–276,714 for Tokio. The busy ranges are 639,707–652,884, 660,179–675,519, and 701,315–718,073 respectively. Native serial samples have more variation than Tokio; these results do not establish parity in every workload.

Serial P99 changes from 5.611 to 4.771 µs; Tokio is 4.283 µs. Busy P99 changes from 53.503 to 52.351 µs; Tokio is 52.639 µs. See `lean-confirmation.json` for individual samples and executable hashes.

## Wider comparison

Three repeats across eight worker, connection, and payload configurations compare owned and borrowed Native I/O with raw Tokio and both Tokio adapter forms. Rates are medians of validated echoes per second.

| Workers / connections / bytes | Baseline owned | Candidate owned | Candidate borrowed | Tokio | Adapter owned | Adapter borrowed | Owned change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 / 1 / 64 | 260,346 | 257,561 | 271,947 | 275,939 | 270,360 | 273,735 | -1.07% |
| 1 / 1 / 4096 | 236,159 | 238,475 | 239,643 | 241,265 | 238,664 | 238,947 | +0.98% |
| 1 / 32 / 64 | 647,192 | 663,109 | 635,092 | 706,767 | 648,762 | 644,121 | +2.46% |
| 1 / 32 / 4096 | 574,237 | 601,085 | 564,796 | 635,529 | 581,223 | 585,537 | +4.68% |
| 4 / 1 / 64 | 263,296 | 264,038 | 270,204 | 275,018 | 270,083 | 271,038 | +0.28% |
| 4 / 1 / 4096 | 232,705 | 235,588 | 237,024 | 241,345 | 238,632 | 238,159 | +1.24% |
| 4 / 32 / 64 | 1,707,119 | 1,725,418 | 1,747,050 | 1,738,540 | 1,704,808 | 1,705,654 | +1.07% |
| 4 / 32 / 4096 | 1,590,158 | 1,574,055 | 1,588,213 | 1,617,303 | 1,587,069 | 1,592,721 | -1.01% |

Owned changes range from −1.07% to +4.68%; borrowed changes range from −1.70% to +1.39%. The serial 64-byte result reverses the seven-repeat confirmation, showing that its smaller throughput change is sensitive to sample variation. The busy single-worker improvement repeats at both payload sizes, but the complete matrix does not establish an across-the-board speedup. See `lean-wide.json` and `summary.json` for every sample and range.

The Tokio adapter remains about 8% behind raw Tokio in the busy single-worker cases, for both owned and borrowed buffers. The wrapper's 5% goal is not met. Native's remaining busy single-worker gap is about 5–7%; neither path has established Tokio parity there.

The fresh profiles in `iouring-self.txt`, `tokio-self.txt`, and `profile-manifest.json` put Native capacity admission/return, completion wake registration, task dispatch, and publication on the remaining userspace path. Queue popping alone is a small share; these data do not justify replacing the task implementation without a causal experiment. Perf's unresolved kernel-labelled entries in the userspace profile should not be interpreted as a reliable kernel breakdown.

## Selected changes

The bound resource already borrows its operation lane exclusively. Admission observes the previous guard's release with an acquire load, then stores the new running flag without an atomic exchange. Direct, Tokio, and the readiness selector use the same reasoning.

Normal io_uring scalar result collection returns the guard's activity reference to its own lane for reuse. It can skip a retirement wake because close cannot concurrently borrow that lane. Abandonment still clears the flag and wakes close from the guard's destructor. This adds one optional Arc field per io_uring operation lane and removes repeated reference-count changes after warmup.

The scalar request caches its socket owner between reads. Its waiter borrows the original controller from the selector's stable controller vector rather than cloning the controller for routing and cancellation. Accepted requests keep their own socket and buffer ownership, and cancellation continues to target the accepting ring after migration. A cached socket is replaced when the request is reused for another socket and released when the cache is dropped.

When a single worker publishes to its own ready queue, it can return without searching for an idle worker. It is already executing the callback and checks that queue before waiting again. Remote wakes and multi-worker distribution retain their existing notification paths.

For a successful warmed scalar read, source-level read-plumbing RMWs fall from 28 to 17: two activity reference-count operations, one lane exchange, two unused retirement-waker operations, four controller reference-count operations, and two socket reference-count operations disappear. This count excludes task scheduling, queue fences, kernel work, cancellation, retries, and first-use allocation. It is a source inventory, not a cycle allocation.

## CPU mechanism check

Three separate instrumented repeats of the busy case count server-process CPU events. The client has zero warmup for these runs, so startup and shutdown are included. Their rates are not capacity results.

| Per echo, median | Baseline | Candidate | Tokio |
| --- | ---: | ---: | ---: |
| Userspace cycles | 1,542 | 1,292 | 663 |
| Kernel cycles | 6,712 | 6,831 | 6,858 |
| Userspace instructions | 2,505 | 2,488 | 1,441 |
| Kernel instructions | 12,987 | 12,986 | 12,850 |

Userspace cycles fall about 16% with little change in instruction count. Together with the instruction-level profile's concentration around atomic operations, this supports reducing synchronization stalls as the useful mechanism. The residual busy-case gap is still largely userspace work; it cannot be assigned wholly to async-task from these measurements. See `lean-cpu.json` and `cpu-causal-controls.json`.

## Rejected and diagnostic experiments

All screens retain their complete sample sets and build manifests. Their candidates are exploratory patches against their recorded bases, not additional production changes.

- `wake-screen.json`: putting the completion waker under the existing result mutex gave no consistent improvement. Retain the existing AtomicWaker.
- `handoff-screen.json`: inline polling, direct retirement, mutex-waker variants, and ownership reuse produced mixed results. Only the combined selected change has the separate seven-repeat confirmation above; do not attribute its gain to each component individually.
- `compact-exclusive-screen.json`: outlining receive-ahead did not shrink the owned task and regressed its serial screen. Boxing rare readiness/write retries reduced the task from 1,240 to 1,016 bytes, with +1.72% serial and +0.76% busy medians in one screen. It adds allocation on those retry paths and was not selected.
- `local-lane-screen.json`: separate lane and local-wake candidates are small and mixed. Their combination improves the serial screen, but requires the final combined confirmation rather than extrapolating individual gains.
- `codegen-screen.json`: thin LTO and one codegen unit do not close the gap when Tokio uses the same compiler settings. All ordinary runs already use release optimization level 3.
- `inq-screen.json`: a C control explores conservative persistent polling, a TCP_INQ hint, and a Tokio-like short-read heuristic. The hint gains about 2.08% under load and loses 0.71% serially relative to conservative polling. It is not implemented in the scheduler or validated for urgent-data semantics. The short-read control is deliberately diagnostic and must not be treated as the production stream contract.
- `cpu-causal-controls.json`: a minimal C completion loop is faster than both runtimes while omitting their task, ownership, cancellation, and lifecycle machinery. It rules out an inherent io_uring kernel throughput ceiling at the observed Native rate; it is not a compatible scheduler implementation.

## Reproduction and validation

Aiko is a Ryzen 9950X3D host running Linux 7.2.8, Arch Rust 1.99.0, and LLVM 23. Release builds use opt-level 3, debug 1, 16 codegen units, no LTO, no incremental compilation, and empty Rust flags unless a compiler experiment explicitly overrides them. Compiler and executable identities are in the build manifests. The pre-merge lazy-quiescence source used by the frozen baseline is runtime-equivalent to `f5dbd04`; `git diff f5dbd04 ed607d5 -- src crates benchmarks/echo/src` is empty.

The server is pinned to CPUs 0 through workers−1. The fixed four-worker validating Tokio client is pinned to CPUs 4–7, reuses a 64 KiB buffer, uses TCP_NODELAY, and keeps one outstanding echo per connection. Capacity samples warm up for one second and measure for three seconds. Host power policy is unchanged. Every build, test, and benchmark serializes with `/tmp/core-scheduler-perf.lock`.

The scripts preserve the historical absolute worktree/cache paths used on Aiko. Adapt those paths before reproducing elsewhere. They use the repository's `benchmarks/echo/results/idle-search/wait-affinity.py` and `benchmarks/echo/run.py`; retain the frozen client to avoid changing both ends of the comparison. Source patches and build manifests identify every prototype. The C controls use the source and compiler settings recorded in `inq-build-manifest.json`.

At the selected source commit, full workspace region coverage is 39,193/39,193 on Linux and 26,806/26,806 on macOS. Regression tests cover cross-thread admission/retirement, cached-lane reuse, abandoned close waits, cached socket replacement and destruction, abandoned request retention, and allocation-free warmed reads. Linux formatting, Clippy, and documentation gates also pass. GitHub CI supplies the Windows and minimum-Rust checks before merge.
