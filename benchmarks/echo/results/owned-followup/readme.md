# Owned receive follow-up

This research starts from the runtime merged in [#46](https://github.com/core-rs/core-scheduler/pull/46). Every candidate is a diagnostic patch, not a production change. Small bookkeeping changes have not closed Native's remaining busy single-worker gap with ordinary Tokio.

The source base is `83433057fc469e076dcd7e7f42a61b8147a91ced`. Its runtime and echo sources match merged commit `4bd8b58d1b84226558661a082773a3e39127998f`. The frozen `lean-final` executable is SHA-256 `8ea109bf21845f3fa0aeee9964726a242b65dce08fe4262fe83e664f38d6c12b`. The preceding [selected measurements](../owned-handoff/readme.md) retain the broader owned/borrowed comparison, CPU counts, profiles, and validation of #46.

## Isolating the Tokio adapter's receive policy

The adapter retains readable readiness until a receive returns `WouldBlock`. Ordinary Tokio clears readiness after a successful short read on the measured platform. In a separate three-repeat syscall count, ordinary Tokio makes approximately one receive and one send per echo, while both adapter buffer forms make two receives and one send. See [the raw counters](adapter-syscalls.json) and [profiles](adapter-profiles/manifest.json). Instrumented rates are excluded from capacity comparisons.

The stronger policy control changes only Tokio's successful-short-read readiness clearing condition to false. It preserves `TcpStream`, `AsyncRead`, the Tokio runtime, and the rest of the library. The [one-line library patch](tokio-no-short.patch), build script, and source hashes identify this diagnostic precisely. Earlier controls also changed readiness APIs; this control avoids that confound.

Five repeats per case give these median validated echoes per second:

| Workers / connections / bytes | Ordinary Tokio | Tokio with short-read clearing disabled | Borrowed Tokio adapter |
| --- | ---: | ---: | ---: |
| 1 / 1 / 64 | 275,119 | 274,093 | 271,361 |
| 1 / 32 / 64 | 698,419 | 666,024 | 652,904 |

In the busy case, the adapter is 6.52% below ordinary Tokio and 1.97% below the conservative library control. The policy change itself lowers Tokio's median by 4.64%. This attributes much of this adapter gap to receive policy; it does not establish ordinary-Tokio parity or explain Native's owned-read overhead. The production policy remains unchanged because successful short reads do not always prove that the receive queue is drained, including at an urgent-data boundary.

A Linux `TCP_INQ` prototype asks the kernel whether data remains queued. It improves busy adapter medians by 2.62% for owned buffers and 3.49% for borrowed buffers, while lowering serial medians by 1.31% and 0.96%. It still trails ordinary Tokio by 6.57% and 5.50% in the busy case. It also changes socket-wide ancillary metadata handling and needs more platform and error-path validation. It is not selected. See [the patch](tcp-inq-readiness.patch) and [complete screen](tcp-inq-screen.json).

## Native bookkeeping screens

Each row is a separate five-repeat experiment against the same frozen merged-runtime executable. Changes are ratios of medians within that experiment. They must not be added together. Serial samples vary more than busy samples.

| Experiment | 1 worker / 1 connection | 1 worker / 32 connections | 4 workers / 32 connections |
| --- | ---: | ---: | ---: |
| Retain an inactive capacity permit in the cached read request | +2.25% | +0.42% | −0.68% |
| Replace the read result mutex with an atomic-phase completion slot | +0.54% | +0.40% | −0.33% |
| Try a direct nonblocking receive before the owned ring read | −10.24% | −4.70% | −0.27% |
| Omit runnable-count updates in a single-worker diagnostic | +0.48% | −0.39% | Not measured |
| Retire a frozen runnable batch with one counter subtraction | −0.18% | +0.23% | +0.69% |
| Combine erased submission/completion callbacks, corrected version | +1.29% | +0.07% | +0.17% |
| Flatten the nested owned-read attempt future | +2.13% | +0.08% | +1.55% |

A further mutex-waker prototype retains the ready flag to avoid cloning a waker on completion, moves registration under the result mutex, and collects the result in the same lock acquisition that observes completion. It removes the separate atomic waker, but changes throughput by −0.91% serially, −1.97% with one busy worker, and +1.09% with four workers. The earlier [mutex-waker screen](../owned-handoff/readme.md) cloned even on ready completion; this version avoids that cost but still provides no busy single-worker gain. It passed the existing 240 io_uring unit tests.

None provides a convincing busy single-worker gain. Retaining the existing implementation avoids adding unsafe completion-slot code or a reusable-permit state machine without a demonstrated useful benefit. The direct receive probe is a clear regression. The flattened future preserves ownership but does not improve the workload motivating this investigation.

The runnable-count control deliberately weakens drain semantics: it waits for all tasks rather than runnable quiescence. It is evidence only and must not be shipped. Its lack of a gain does not justify removing shutdown accounting. The first callback-dispatch patch also changed retention order; only `request-dispatch-v2.patch` preserves the original in-flight insertion and ownership order. Neither version is selected.

The completion-slot prototype passed existing unit tests, but has no new concurrency model or race coverage. The `TCP_INQ` prototype passed the existing urgent-data regression and required ancillary-metadata adjustments. Existing test success is insufficient to promote either prototype into production. Build and test logs remain on Aiko beside the frozen executables.

## Task execution and submission controls

A separate-driver experiment feeds the same owned io_uring socket operations through either Native's executor or Tokio's executor. Both use one dedicated ring-driver thread, and all server threads share the case's CPU affinity. Five-repeat medians are:

| Workers / connections / bytes | Separate-driver Native | Separate-driver Tokio | Integrated Native | Ordinary Tokio |
| --- | ---: | ---: | ---: | ---: |
| 1 / 1 / 64 | 191,691 | 172,751 | 267,341 | 273,624 |
| 1 / 32 / 64 | 456,055 | 451,118 | 668,824 | 707,025 |
| 4 / 32 / 64 | 1,432,800 | 1,053,581 | 1,716,623 | 1,762,249 |

Native does not trail Tokio in this common-I/O topology. The experiment adds a driver thread and remote wakes, so it does not isolate the cost of the integrated task implementation or prove that its overhead is zero. It does argue against replacing `async-task` solely on the assumption that it explains the whole echo gap. See [the patch](split-executor.patch) and [matrix](split-screen.json).

A task-only benchmark uses the same future on both executors, excludes its initial gate poll from timing, and validates exactly five million polls per sample. The ordinary mode calls `wake_by_ref`; the owned-wake mode clones the waker and consumes that clone. Five-repeat single-worker medians are:

| Tasks | Wake form | Native polls/s | Tokio polls/s | Native ns/poll | Tokio ns/poll |
| --- | --- | ---: | ---: | ---: | ---: |
| 1 | Borrowed | 20,979,664 | 23,842,141 | 47.67 | 41.94 |
| 32 | Borrowed | 24,319,606 | 23,861,025 | 41.12 | 41.91 |
| 1 | Owned | 17,940,849 | 21,588,739 | 55.74 | 46.32 |
| 32 | Owned | 20,460,654 | 21,636,320 | 48.87 | 46.22 |

Native is slightly ahead for 32 borrowed-wake tasks and about 5.43% behind with owned wakes; the latter difference is about 2.66 ns per poll. The single-task path trails more. These are task/executor costs for self-wakes during a poll, not isolated task-primitive costs or an equivalent model of completions arriving between polls. They do not directly predict echo throughput. See [the source](scripts/task-poll.rs) and [raw samples](task-poll-current.json).

Loading async-task's current state immediately after a pending future poll, rather than starting its state-update loop from the cached pre-poll state, lowers Native throughput by 2.25% serially, 1.31% in the busy single-worker case, and 1.94% with four workers. Extra loads do not establish a useful improvement over occasional compare-exchange retries. This cached-registry-source diagnostic changes no production task dependency.

Bypassing scalar-read capacity admission entirely improves the busy median by only 0.83%, with +0.63% serial and −0.79% four-worker results. This deliberately violates admission limits and cannot be shipped. Profile percentages are not predictions of the gains from deleting the corresponding code.

Submitting partial ready batches after every four tasks lowers busy throughput by 8.01%; after sixteen tasks, it changes throughput by −0.08%. Each control preserves ready-list generations but adds two zero-time driver turns at its selected boundary, ensuring that staged work reaches submission. The measured result supports retaining the current batch boundary.

## Future construction and compiler controls

| Experiment | 1 worker / 1 connection | 1 worker / 32 connections | 4 workers / 32 connections |
| --- | ---: | ---: | ---: |
| Box the receive-ahead fallback in owned reads | +0.95% | −2.20% | +0.17% |
| Also box owned-write and read-readiness retries | −1.56% | −2.47% | +1.15% |
| Attempt the immediate send before constructing its waiting future | −0.57% | −1.50% | +0.28% |

All three prototypes passed the existing io_uring unit tests. None improves the busy single-worker workload. Boxing also adds allocation on the selected branches, and is not selected. Splitting the write probe adds another private function without a demonstrated benefit, and is not selected either.

Disabling compiler SLP vectorization improves Native's serial median by 2.45% but lowers its busy median by 1.33% and its four-worker median by 0.42%. The same compiler setting changes Tokio by −1.06%, +0.87%, and +0.16% respectively. This broad compiler control is not an isolated measure of aggregate-buffer copy costs: other lowering can still emit SIMD copies.

Adding a normal `#[inline]` hint to operation-lane admission changes serial throughput by +0.30%, busy single-worker throughput by −1.49%, and four-worker throughput by +0.51%. This narrow compiler hint is not selected.

The [instruction-level annotation](native-echo-annotate.txt) contains samples around buffer-owner movement and synchronization. Sampling skid and memory stalls prevent interpreting those percentages as directly removable copy costs. Smaller future layouts and additional outlining have not yielded a convincing busy-case gain in these controls.

Keep performance changes focused on eliminating work and redundant state. A more complex completion slot, special allocator, or custom task implementation needs a clear causal measurement and a repeatable gain under the same ownership, cancellation, and shutdown contracts. This archive records rejected experiments to avoid repeating them; it does not select a new runtime change.

## Fusing a task transition

A second async-task diagnostic folds the ordinary pending task's runnable-reference release into the compare-exchange which marks it idle. The closed and rescheduled paths retain their original handling. This removes one atomic read-modify-write from that normal path rather than adding a load or a new cache.

Five-repeat echo medians change by +3.55% serially, +0.67% with one busy worker, and −0.11% with four workers. Serial baseline rates vary substantially in this screen; these samples do not establish a repeatable serial gain. The busy change is small, and Native still trails Tokio there by 5.36%. No production task dependency or new task crate is selected.

The diagnostic passes 79 existing Native unit tests and the upstream task integration tests, including wake/drop/cancellation races. Those tests do not establish complete concurrency correctness for a production fork. See [the patch](async-task-fused-release.patch), [matrix](fused-release-screen.json), and [test logs](logs).

## Connection count and cache counters

A separate three-repeat single-worker sweep uses the unchanged frozen executable for Native and Tokio:

| Connections | Native echoes/s | Tokio echoes/s | Native vs Tokio |
| --- | ---: | ---: | ---: |
| 1 | 266,716 | 274,468 | −2.82% |
| 8 | 619,651 | 694,450 | −10.77% |
| 32 | 661,815 | 701,577 | −5.67% |
| 128 | 658,162 | 703,366 | −6.43% |

The gap is larger at eight connections than at 32. These data do not support a simple explanation in which Native's larger per-task working set causes a growing throughput deficit as connections increase.

Two separate cache-counter repeats per configuration record userspace events. At 32 connections, Native reports about 1,292 userspace cycles, 2,487 instructions, and 50.7 L1 data-load misses per echo; Tokio reports about 664 cycles, 1,441 instructions, and 10.4 misses. Native's data-load misses are roughly 51.1 at eight connections and 49.5 at 128, rather than increasing with task count. The instruction difference and additional data traffic support examining the whole I/O handoff, but these aggregate counters cannot identify the objects or code responsible. They do not prove that task-future size or any particular copy causes the gap.

Cache-counter runs have no warmup and include process startup and shutdown. Their timing is diagnostic: the single-connection instrumented rates differ markedly from capacity rates. Do not compare those rates as an optimization result. The raw CSVs retain the event-running percentages, and the JSON records counts normalized by validated echoes. See [the capacity sweep](concurrency-screen.json) and [cache counters](concurrency-cache.json).

## Method and reproduction

Aiko runs Linux 7.2.8 on a Ryzen 9950X3D, with Rust 1.99.0 and LLVM 23.1.1. Builds use optimization level 3, debug information level 1, 16 codegen units, no LTO, no incremental compilation, and empty Rust flags. The manifests preserve exact binary and patch hashes. A clean unpatched rebuild reproduces the original `lean-final` executable byte for byte; the harness rejects a redundant timing comparison of those identical executables. See [the clean rebuild manifest](clean-rebuild-build-manifest.json). Capacity samples use one second of warmup and three measured seconds, deterministic shuffled order, persistent IPv4 TCP connections, `TCP_NODELAY`, reused 64 KiB server buffers, and one outstanding validated echo per connection.

Server threads inherit CPUs 0 through workers−1; the fixed four-worker client uses CPUs 4–7. Its SHA-256 is `a8c84568e0932f782179fbbb1c64272129c6af9a5f1344a8c84998b22527fc5f`. All builds, tests, timings, and profiling serialize on `/tmp/core-scheduler-perf.lock`. Host power policy is unchanged.

The [scripts](scripts) preserve historical Aiko paths. Adapt those paths before reproducing elsewhere. Build scripts apply one patch to the clean source base, freeze the executable and manifest, then restore tracked files. The Tokio policy experiment patches a cached registry source copy and restores Cargo's lockfile after its temporary path override. No patch is cumulative with another experiment.

Run `python3 benchmarks/echo/results/owned-followup/summarize.py` to validate report completeness, affinities, measurement durations, patch/build hashes, and executable identities, and regenerate [the complete summary](summary.json). The archive retains every sample, including the low busy baseline in the batch-retirement screen. Five short repeats do not establish universal sub-percent equivalence or predict physical-network and general application performance.
