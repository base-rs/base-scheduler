# Native monitor polling

This investigation targets `NativeScheduler<IoUringSelector>`. The selected change retries borrowed monitor I/O directly on each task poll and consults readiness only after `WouldBlock`. It removes the nested readiness-wait future from the I/O retry loop. Public capabilities, borrowed-buffer lifetimes, persistent kernel polls, and conservative readiness clearing remain unchanged.

## What changed

Previously, resuming a pending receive first polled `monitor_wait`: check cancellation and closure, clone a replacement waker, lock registration status, observe readiness, release the waiter, then retry the syscall. The selected implementation checks cancellation, closure, and monitor direction, then tries the nonblocking syscall. Successful I/O returns immediately. Only `WouldBlock` clears the observed readiness generation and enters readiness bookkeeping.

A shared private `poll_monitor_wait` helper serves both explicit readiness waits and I/O operations. A pending I/O operation retains one waiter claim and the route selected by its first wait until it completes or is dropped. Moving the future between workers does not change that active route. A later operation can retire and rebind the persistent registration. No payload pointer enters the ring, and dropping a pending operation still removes its waker and releases its claim.

Cancellation observed before a syscall prevents it. A syscall that consumes data returns its result in that poll, including when cancellation races the syscall. Interrupted calls retry with preflight checks. Readiness remains cached until `WouldBlock`; this preserves progress across TCP urgent-data boundaries. An independently repolled future can now consume available data before its readiness CQE has been processed, so tests that specifically inspect CQE effects drive the driver explicitly.

## Final comparison

The final [120 samples](results/native-monitor/preflight.json) compare the selected candidate, unmodified Native, and raw Tokio with five repetitions of each configuration. Entries are median round trips/second; parentheses show observed minimum–maximum. The measured source patch, before the final explanatory comment and test updates, is [preflight.patch](results/native-monitor/preflight.patch).

| Workers | Connections | Payload | Native before | Native after | Change | Tokio |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 64 B | 273,254 (271,248–274,436) | 274,761 (272,944–279,313) | +0.55% | 276,259 (273,298–279,240) |
| 1 | 1 | 4096 B | 240,484 (238,170–244,805) | 244,939 (241,673–248,716) | +1.85% | 243,348 (238,162–247,293) |
| 1 | 32 | 64 B | 628,295 (626,236–629,632) | 637,443 (636,424–643,092) | +1.46% | 714,071 (707,729–716,740) |
| 1 | 32 | 4096 B | 570,072 (566,165–573,617) | 580,940 (575,065–583,323) | +1.91% | 637,447 (629,318–645,256) |
| 4 | 1 | 64 B | 263,618 (245,093–268,903) | 262,715 (238,523–268,327) | -0.34% | 274,763 (274,010–278,765) |
| 4 | 1 | 4096 B | 242,843 (241,704–244,664) | 243,736 (241,618–246,279) | +0.37% | 243,493 (242,110–245,455) |
| 4 | 32 | 64 B | 1,743,829 (1,715,526–1,761,804) | 1,742,607 (1,730,861–1,762,662) | -0.07% | 1,758,313 (1,730,813–1,764,735) |
| 4 | 32 | 4096 B | 1,599,877 (1,575,973–1,622,504) | 1,602,975 (1,587,029–1,635,988) | +0.19% | 1,626,913 (1,584,821–1,661,753) |

The geometric mean of the eight candidate/baseline median ratios is **+0.74%**. The clearest gains are the one-worker/32-connection cases: **+1.46% at 64 bytes and +1.91% at 4 KiB**. Four-worker changes are small compared with the observed variation, particularly the single-connection 64-byte case. This is a small improvement, not a general performance breakthrough.

The remaining gap to raw Tokio is **10.7% at 64 bytes and 8.9% at 4 KiB** with one worker and 32 connections. With four workers and 32 connections, Native is within approximately 0.9–1.5% in this run. Single-connection results are mostly similar, except the noisy four-worker/64-byte case where Native remains 4.4% behind by median. These comparisons describe the tested workload rather than an executor-wide ranking.

## Experiments

The focused experiments use one worker, 32 connections, and five repetitions per payload. Percentages compare Native candidate and baseline medians from the same run; they are not additive. The first monitor simplifications were not combined into the selected implementation.

| Experiment | 64-byte throughput change | 4 KiB throughput change | Decision |
| --- | ---: | ---: | --- |
| Publish the readiness generation with a load/store under the existing notification mutex | −0.01% | −0.07% | No measured benefit; omitted. |
| Also track whether waiter cleanup still needs the status lock | +1.39% | +0.27% | Small result; omitted the extra tracking state. |
| Also inline the small monitor helpers | +0.41% | +1.05% | No clear additional benefit; omitted. |
| Retry the syscall before polling the nested readiness future | +1.45% | +0.83% | Developed into the shared polling helper. |
| Submit every owned scalar send through io_uring | −16.33% | −15.17% | Rejected. |

The owned-send experiment removes the synchronous send attempt but introduces another request allocation, capacity acquisition, submission, completion, and task suspension per successful echo. Fewer direct syscalls do not by themselves make this path faster. The existing synchronous send fast path remains in place. The experiment is measured on the `iouring` backend, while the monitor experiments use `iouring-monitor`.

An intermediate [120-sample monitor run](results/native-monitor/fused.json), before removing repeated preflight checks, improved the focused 64-byte and 4 KiB medians by 0.97% and 1.47%. Its four-worker/single-connection/64-byte median decreased 1.90% amid a wide run range. That variability is a reason to retain the full matrix and raw samples, rather than extrapolate from the focused case.

## Reproduction and limits

The base is `45fb36e`, with identical echo benchmark source and locked dependencies for every executable. Build with `CARGO_PROFILE_RELEASE_DEBUG=1 cargo build --release --locked -p core-scheduler-echo-benchmark`. This is the optimized release profile with debug information, without LTO or target-CPU overrides. Patches and JSON reports for every experiment are under [results/native-monitor](results/native-monitor).

Aiko uses a Ryzen 9 9950X3D, Linux 7.2.8, and Rust 1.99.0. Server affinity is CPU 0 for one worker and CPUs 0–3 for four; the fixed four-worker client uses CPUs 4–7. These are separate physical cores in the same L3 group. The original client binary is unchanged, with SHA-256 `c72f7aea7151fb83020b096cfdaccc46a70b0eeff6bbb0a0e4ea4157d1aeb071`.

[compare.py](results/native-monitor/compare.py) reuses the existing echo runner's sampling function and records server/client hashes, source patch, toolchain, CPU topology, load, latency percentiles, and every sample. It runs each case five times with a one-second warmup and three-second measurement, shuffling case and executable order with a recorded seed. Preserve each built server executable and pass `label:backend:binary` entries through `--variants`. The script records the Aiko source and fixed-client paths used here. Builds, benchmarks, and test runs serialize on `/tmp/core-scheduler-perf.lock`.

These are closed-loop loopback echo measurements with one outstanding request per connection, reused 64 KiB buffers, `TCP_NODELAY`, complete partial writes, and validated payloads. They do not establish server saturation, physical-network performance, or a universal speedup. Run ranges overlap in several cases; small changes should be read as modest evidence, not precise guarantees. Native's remaining one-worker gap to raw Tokio is not solved by this change. Tokio's short-read readiness policy and Native's conservative `WouldBlock` policy still differ, as discussed in the [readiness investigation](readiness-performance.md).
