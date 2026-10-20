# Worker-loop and timer follow-up

The empty admission-queue check is a small, repeatable Native improvement. A separate clean timer-queue shortcut reduces isolated timer costs without changing the architecture. The implementation includes both changes. The [worker hot-path report](worker-hotpath-research.md) records the executor measurements; [the follow-up evidence](results/worker-loop-followup/readme.md) records the timer measurements and fresh profiles.

## Avoid flushing an unchanged timer queue

`State::flush` now returns immediately when there are no scheduled insertions or cancellations. This removes redundant bookkeeping and avoids moving the pending buffer during individual expiry. Every actual mutation still follows the existing flush and publication path. There is no additional state, allocation, public API, or locking.

Nine randomized repetitions of the optimized, CPU-pinned primitive benchmark give:

| Production shared queue | Before median | After median | Elapsed change |
| --- | ---: | ---: | ---: |
| 4 million populated deadline queries | 42.102 ms | 39.604 ms | −5.93% |
| 2.048 million schedule/expire/release operations | 119.895 ms | 117.926 ms | −1.64% |

The before/after ranges do not overlap for either primitive. The whole scheduler timer matrix, with three repetitions per cell, is essentially unchanged. For example, four workers with 1,024 relative 1 ms timers use a median 5.688 versus 5.698 process CPU seconds and deliver 1.015 million expiries/s in either case. This is a modest queue efficiency improvement, not evidence of a broad scheduler speedup.

The timer package passes Bake's coverage gate on macOS and Linux: 584/584 default regions, 1,064/1,064 driver-only regions, 1,017/1,017 sharded-only regions, and 1,491/1,491 all-feature regions. Linux's compiler has remapped standard-library sources unavailable on the host; the archived LLVM wrapper excludes only `/usr/src/debug/` from the source report, retaining all Core source regions and the original filters. The feature checks use isolated coverage targets after a first Linux invocation picked up unused sharded instrumentation from an earlier Native dependency build; the failed invocation and successful isolated checks are recorded in `timer-feature-verification.json`. Windows CI was not run during this local study.

## Worker-local timers remain an experiment

The earlier local-timer prototype has much lower timer CPU costs than the production shared fallback, but still has unresolved capacity, conformance, and mixed-workload tradeoffs. It is not applied to production.

Applying the clean-queue shortcut and removing redundant flush/deadline work from individual local expiry lowers isolated local deadline-query cost by 54.4% and schedule/expire/release cost by 22.8%. Those large primitive reductions do not transfer uniformly to the scheduler. Periodic timers use about 3.2% less process CPU, synchronized expiry uses about 1.7% more, and some p99 lateness measurements worsen. These local changes remain archived research.

A separate local-prototype patch fixes capacity retention after cancelling non-earliest entries in a small heap: an exhausted queue compacts released cancellations before rejecting new registrations. That patch was correctness-tested, but was not included in these timing results.

## Fresh profiles of the no-timer regression

Frozen binaries reproduce the earlier small no-timer median regression when enabling the original worker-local timer prototype:

| One worker/connection, 64 bytes | Production | Local timers | Throughput change |
| --- | ---: | ---: | ---: |
| Owned I/O | 237,922/s | 233,721/s | −1.77% |
| Borrowed I/O | 239,992/s | 236,895/s | −1.29% |

Ranges overlap widely. Separate `perf stat` runs show about 4.5% more instructions per exchange with local timers, while both variants still make approximately two `io_uring_enter` calls and one context switch per exchange. Cycle changes are much smaller than instruction changes. Kernel TCP and locking dominate the cycle profiles; the samples do not identify one timer helper as the cause of the small throughput difference. Inlined empty-queue work is not fully visible under the timer symbol names.

The admission queues appear among the executor costs. This motivated the independently measured [empty-injector check](worker-hotpath-research.md). The extra ring entry motivated the separately measured driver experiments below.

## Combining idle submission and waiting

The larger driver prototype changes the worker's nonblocking turn order and lets accepted submissions reach the idle wait without an intervening submit-only entry. Preparing a request currently invokes user-provided buffer methods; those methods can wake tasks. Combining submission and waiting safely therefore also requires moving buffer preparation into the caller's first poll, after the request has been boxed, before enqueueing it. Otherwise the driver could sleep after a preparation callback made a task runnable. The prototype preserves original-completion ownership and cancellation, but adds cached scalar pointer/length state and changes request access.

Separate diagnostic counter runs confirm that this changes the small single-connection path from approximately two to one `io_uring_enter` calls per completed exchange. Instructions and cycles fall too. Uninstrumented capacity runs show:

| Workers/connections/bytes | Owned change | Borrowed change |
| --- | ---: | ---: |
| 1/1/64, initial screen | +6.33% | +8.52% |
| 1/1/64, separate confirmation | +5.30% | +6.33% |
| 1/32/64, five-repeat confirmation | +0.06% | −0.43% |
| 1/1/4096, confirmation | −0.45% | −1.92% |
| 4/128/4096, confirmation | −0.95% | +0.07% |

Adding immediate completion collection after the combined wait improves the small-message single-connection result further: +7.52% owned and +9.93% borrowed. Its full matrix still shows −1.31%/−1.02% at one worker with 32 connections, −0.47%/−1.30% at one connection with 4 KiB messages, and −1.15%/−0.21% at four workers with 128 connections and 4 KiB messages. These small regressions are workload-dependent; three repeats cannot establish sub-percent differences precisely.

Both combined-wait variants remain archived experiments. The small-message win is real enough to merit future work, but does not yet justify the added buffer-preparation machinery and broader tradeoffs. The production patch does not change scalar request layout or submission semantics. Focused Miri models support the corrected preparation order, but do not establish an existing scalar aliasing defect or replace runtime ownership tests.

The first combined-wait prototype passes Linux Native and io_uring coverage (1,337/1,337 and 2,128/2,128). The combined wait plus immediate collection prototype reaches 2,138/2,139 io_uring regions: its post-wait notification-read error path was not covered in that archived variant. It is not a release-ready implementation. The separately tested minimal collection variant adds deterministic regression coverage for that path.

## Collecting completions immediately after waiting

The final isolated variant only extracts the existing CQ snapshot publication into a helper and calls it again after a blocking wait. It retains the original worker order, submission behavior, request preparation, and ownership representation. This tests whether eliminating the extra empty worker search and driver turn is worthwhile by itself.

Three randomized repetitions of each workload give:

| Workers/connections/bytes | Owned before → candidate | Borrowed before → candidate | Unchanged Tokio |
| --- | ---: | ---: | ---: |
| 1/1/64 | 247,367 → 246,074 (−0.52%) | 238,036 → 236,697 (−0.56%) | 270,381 |
| 1/32/64 | 637,693 → 633,831 (−0.61%) | 575,563 → 578,263 (+0.47%) | 703,599 |
| 4/128/64 | 2,083,525 → 2,097,062 (+0.65%) | 2,122,824 → 2,127,085 (+0.20%) | 2,132,396 |
| 1/1/4096 | 228,477 → 236,129 (+3.35%) | 233,016 → 235,591 (+1.11%) | 235,583 |
| 4/128/4096 | 1,865,674 → 1,868,036 (+0.13%) | 1,870,368 → 1,888,937 (+0.99%) | 1,865,965 |

Most ranges overlap. Even the largest owned improvement has overlapping ranges: 226,361–234,705 before and 233,615–236,516 after. This screen does not establish a consistent broad improvement. The variant passes Linux Native and io_uring coverage at 1,337/1,337 and 2,131/2,131 regions, with formatting and all-target/all-feature Clippy passing. It remains an archived experiment, rather than an applied production change.

The final applied source changes are the empty admission-queue guard, its idle remote-wake regression test, and the clean timer-flush guard. None changes the public scheduler interface. The main remaining driver opportunity is combining submission with waiting without adding fragile preparation state or worsening busy and larger-message workloads; simply moving completion collection is not a demonstrated substitute.

## Measurement scope

All Aiko builds, tests, timing, and profiling take the exclusive performance lock. Builds use locked dependencies and the default optimized Cargo release profile, without LTO or target-CPU overrides. The timer and timer-profile studies use matching 1 ns process timer slack. Servers are CPU-pinned; echo clients use the same frozen client binary and persistent validated IPv4 TCP connections with `TCP_NODELAY`.

Primitive workloads use `black_box` around both the mutable queue and returned values. Whole scheduler CPU totals cover complete process lifetimes. Echo capacity controls are uninstrumented; profiler throughput is diagnostic only. Short loopback workloads do not predict physical-network or general application performance, and small differences should not be treated as precise universal percentages.
