# Socket readiness performance

The selected changes reserve Tokio's lightweight polling waiter per socket direction, notify io_uring capacity waiters only when admission leaves full capacity, and notify native drain waiters only when the outstanding runnable count reaches zero. The native interrupt and wait implementation remains the original one. The benchmark chooses owned or borrowed echo dispatch before constructing its task future in both comparison builds.

The [direct-ring interrupt prototype](direct-ring-experiment.md) was rejected after its 300-sample comparison exposed a 14.8% serial owned-I/O regression. That report retains the complete matrix, profiles, io-event review, and isolated experiments, including their source revisions and executable hashes. Its native results do not describe the selected implementation.

## Selected implementation

Tokio previously used independent linked-list readiness waiters even with one reader and writer. It now reserves one polling waker per direction until completion or drop. Concurrent operations retain Tokio's independent waiters. Forgetting a reservation leaves later operations on that independent path. The reservation changes no payload ownership or cancellation semantics.

Every operation still attempts its nonblocking syscall before consulting readiness. An experimental readiness-first shortcut delayed valid lifecycle calls and immediate OS errors after a previous operation cleared readiness. Independent review reproduced this with repeated `listen` and invalid `accept`; the shortcut was removed and regression tests added. Lazy registration remains necessary to avoid permanently cached hangup readiness on unconnected Linux sockets.

Capacity and drain notifications occur at the transitions that can release their waiters; closure retains independent notifications. Local worker wake publication is unchanged. No direct-ring idle-wait or eventfd prototype is included in the final runtime.

## io-event review

At [io-event revision c8550a8](https://github.com/socketry/io-event/blob/c8550a85e340941491ee4c09b73d54c5310fed1e/ext/io/event/selector/uring.c#L1726), the selector arms an asynchronous read on its eventfd before releasing the GVL and waiting on io_uring. A waking thread writes that fd without touching SQ/CQ. The read completion consumes the counter, and a sentinel identifies it without a request-table lookup. A registration flag preserves the pending read across timeout or interruption.

This mechanism fits a ring owned by one worker. Ruby's GVL-protected `blocked` check does not replace Rust's atomic publication/parking handshake. Core also retains complete-CQ-snapshot publication to the executor's ready queues; Ruby resumes fibers from its CQ loop. Ring setup flags such as single issuer and deferred task work require a separate review of ring creation and worker ownership.

Independent review found a potential teardown use-after-free: the read targets inline `selector->wakeup_value`, while [close and destruction](https://github.com/socketry/io-event/blob/c8550a85e340941491ee4c09b73d54c5310fed1e/ext/io/event/selector/uring.c#L141-L170) close the ring and free the selector without explicitly draining that read. This is a substantiated missing lifetime guarantee, with no reproduced crash. [Upstream cancellation documentation](https://github.com/axboe/liburing/blob/f4fdfd910ad15e5477c68f2eec208581e3f2239e/man/io_uring_cancelation.7#L241-L249) distinguishes shutdown cancellation from draining before freeing associated resources. A direct read needs destination storage retained through original completion, including exceptional teardown. The pointer-free `PollAdd` prototype avoids that particular obligation. The io-event checkout was not changed.

## Final confirmation

The selected runtime is `938e7e641f8d186991a0f46f211f4ce3d19c929e`. Its repeated optimized comparison uses the same benchmark source in both builds and the same fixed client. The [complete 180-sample final matrix](results/socket-performance/selected/aiko.json) is separate from the rejected prototype.

The four-worker Tokio adapters achieve 95.3–96.3% of pure Tokio throughput, meeting the approximate 5% goal in these configurations. Single-connection adapters are effectively level with Tokio. The one-worker/32-connection adapters remain at 88.8–91.4% of Tokio, so that goal is not met throughout the matrix. Concurrent adapter throughput improves by roughly 2–5% against its same-run before build.

The native owned 64-byte serial regression is removed: 234,080 versus 235,083 round trips/second (−0.4%), with effectively unchanged lifecycle CPU use. Concurrent native owned throughput improves by about 1–3%; borrowed native throughput is mostly unchanged. Four-worker borrowed Native reaches 96.9–97.2% of Tokio, but owned Native remains at 84.9–86.6%, and one-worker cases still have substantial gaps. The native-equals-or-beats-Tokio goal remains future work.

The final serial owned 4 KiB median is 4.9% lower, with wide before variation (150,439–202,513 round trips/second) versus 179,859–194,274 afterward. A separate [15-sample focused follow-up](results/socket-performance/selected/focus-4k/analysis.md), using the same frozen executables, gives a 1.27% lower median but a 2.47% higher mean. Its overlapping ranges and sensitivity to summary statistic do not establish the original drop as reproducible; a small regression remains unresolved. The original matrix and follow-up remain separate, with no samples removed or pooled. The 8.7% single-connection owned Tokio improvement in this matrix also reflects variable before samples (247,081–266,263), compared with 267,913–270,676 afterward; the earlier five-repeat comparison measured 3.5%. Neither number is a universal speedup.

Median complete round trips per second across three repetitions:

| Workers | Connections | Bytes | Backend | Before | After | Change |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 1 | 64 | tokio | 269,225 | 271,540 | +0.9% |
| 1 | 1 | 64 | tokio-adapter | 248,685 | 270,439 | +8.7% |
| 1 | 1 | 64 | tokio-adapter-poll | 264,301 | 270,724 | +2.4% |
| 1 | 1 | 64 | iouring | 235,083 | 234,080 | -0.4% |
| 1 | 1 | 64 | iouring-poll | 181,355 | 180,635 | -0.4% |
| 1 | 1 | 4096 | tokio | 233,957 | 233,434 | -0.2% |
| 1 | 1 | 4096 | tokio-adapter | 232,597 | 235,942 | +1.4% |
| 1 | 1 | 4096 | tokio-adapter-poll | 237,827 | 235,590 | -0.9% |
| 1 | 1 | 4096 | iouring | 197,231 | 187,550 | -4.9% |
| 1 | 1 | 4096 | iouring-poll | 204,380 | 211,062 | +3.3% |
| 1 | 32 | 64 | tokio | 700,121 | 698,873 | -0.2% |
| 1 | 32 | 64 | tokio-adapter | 592,738 | 621,486 | +4.8% |
| 1 | 32 | 64 | tokio-adapter-poll | 596,178 | 620,540 | +4.1% |
| 1 | 32 | 64 | iouring | 515,031 | 530,751 | +3.1% |
| 1 | 32 | 64 | iouring-poll | 560,135 | 562,962 | +0.5% |
| 1 | 32 | 4096 | tokio | 610,346 | 609,056 | -0.2% |
| 1 | 32 | 4096 | tokio-adapter | 535,565 | 550,067 | +2.7% |
| 1 | 32 | 4096 | tokio-adapter-poll | 538,148 | 556,851 | +3.5% |
| 1 | 32 | 4096 | iouring | 468,101 | 476,092 | +1.7% |
| 1 | 32 | 4096 | iouring-poll | 506,746 | 502,733 | -0.8% |
| 4 | 32 | 64 | tokio | 1,734,788 | 1,746,590 | +0.7% |
| 4 | 32 | 64 | tokio-adapter | 1,597,237 | 1,676,622 | +5.0% |
| 4 | 32 | 64 | tokio-adapter-poll | 1,628,684 | 1,664,370 | +2.2% |
| 4 | 32 | 64 | iouring | 1,494,315 | 1,512,171 | +1.2% |
| 4 | 32 | 64 | iouring-poll | 1,695,430 | 1,697,209 | +0.1% |
| 4 | 32 | 4096 | tokio | 1,636,337 | 1,620,951 | -0.9% |
| 4 | 32 | 4096 | tokio-adapter | 1,510,453 | 1,558,468 | +3.2% |
| 4 | 32 | 4096 | tokio-adapter-poll | 1,499,247 | 1,561,650 | +4.2% |
| 4 | 32 | 4096 | iouring | 1,360,776 | 1,375,925 | +1.1% |
| 4 | 32 | 4096 | iouring-poll | 1,570,860 | 1,570,919 | +0.0% |

After-change throughput as a fraction of the same-build pure Tokio control:

| Workers | Connections | Bytes | Tokio owned | Tokio borrowed | Native owned | Native borrowed |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 1 | 64 | 99.6% | 99.7% | 86.2% | 66.5% |
| 1 | 1 | 4096 | 101.1% | 100.9% | 80.3% | 90.4% |
| 1 | 32 | 64 | 88.9% | 88.8% | 75.9% | 80.6% |
| 1 | 32 | 4096 | 90.3% | 91.4% | 78.2% | 82.5% |
| 4 | 32 | 64 | 96.0% | 95.3% | 86.6% | 97.2% |
| 4 | 32 | 4096 | 96.1% | 96.3% | 84.9% | 96.9% |

Minimum–maximum throughput of the unchanged pure Tokio controls:

| Workers | Connections | Bytes | Before control | After control |
| --- | --- | --- | --- | --- |
| 1 | 1 | 64 | 267,658–271,150 | 270,600–272,766 |
| 1 | 1 | 4096 | 233,257–235,669 | 232,530–235,655 |
| 1 | 32 | 64 | 698,119–708,123 | 698,643–705,600 |
| 1 | 32 | 4096 | 608,084–621,173 | 606,855–611,169 |
| 4 | 32 | 64 | 1,732,168–1,740,828 | 1,731,933–1,752,956 |
| 4 | 32 | 4096 | 1,587,103–1,645,027 | 1,606,999–1,636,880 |

Median per-run p50 / p99 round-trip latency, in microseconds:

| Workers | Connections | Bytes | Backend | Before | After |
| --- | --- | --- | --- | --- | --- |
| 1 | 1 | 64 | tokio | 3.58 / 4.35 | 3.55 / 4.31 |
| 1 | 1 | 64 | tokio-adapter | 3.62 / 5.73 | 3.58 / 4.72 |
| 1 | 1 | 64 | tokio-adapter-poll | 3.62 / 5.07 | 3.57 / 4.78 |
| 1 | 1 | 64 | iouring | 4.02 / 5.99 | 4.00 / 6.26 |
| 1 | 1 | 64 | iouring-poll | 5.36 / 6.93 | 5.39 / 6.79 |
| 1 | 1 | 4096 | tokio | 4.06 / 5.38 | 4.00 / 5.39 |
| 1 | 1 | 4096 | tokio-adapter | 3.96 / 5.44 | 3.91 / 5.39 |
| 1 | 1 | 4096 | tokio-adapter-poll | 3.91 / 5.38 | 3.92 / 5.38 |
| 1 | 1 | 4096 | iouring | 4.49 / 7.58 | 5.00 / 7.64 |
| 1 | 1 | 4096 | iouring-poll | 4.96 / 5.95 | 4.44 / 5.88 |
| 1 | 32 | 64 | tokio | 46.69 / 53.47 | 46.75 / 53.47 |
| 1 | 32 | 64 | tokio-adapter | 55.20 / 61.28 | 52.61 / 59.23 |
| 1 | 32 | 64 | tokio-adapter-poll | 54.85 / 61.15 | 52.67 / 59.39 |
| 1 | 32 | 64 | iouring | 61.53 / 70.21 | 59.74 / 67.84 |
| 1 | 32 | 64 | iouring-poll | 56.86 / 63.23 | 56.61 / 62.75 |
| 1 | 32 | 4096 | tokio | 53.60 / 60.99 | 53.73 / 61.02 |
| 1 | 32 | 4096 | tokio-adapter | 61.09 / 67.90 | 59.49 / 66.37 |
| 1 | 32 | 4096 | tokio-adapter-poll | 60.80 / 67.71 | 58.75 / 65.79 |
| 1 | 32 | 4096 | iouring | 67.78 / 76.80 | 66.69 / 75.39 |
| 1 | 32 | 4096 | iouring-poll | 62.88 / 69.44 | 63.39 / 69.82 |
| 4 | 32 | 64 | tokio | 17.42 / 52.64 | 17.38 / 52.41 |
| 4 | 32 | 64 | tokio-adapter | 18.78 / 54.24 | 17.90 / 52.83 |
| 4 | 32 | 64 | tokio-adapter-poll | 18.59 / 53.57 | 17.93 / 52.96 |
| 4 | 32 | 64 | iouring | 20.06 / 56.22 | 19.74 / 56.26 |
| 4 | 32 | 64 | iouring-poll | 17.49 / 54.85 | 17.44 / 55.13 |
| 4 | 32 | 4096 | tokio | 18.73 / 59.26 | 18.77 / 59.90 |
| 4 | 32 | 4096 | tokio-adapter | 20.03 / 60.58 | 19.30 / 59.81 |
| 4 | 32 | 4096 | tokio-adapter-poll | 19.90 / 60.64 | 19.34 / 59.84 |
| 4 | 32 | 4096 | iouring | 21.97 / 64.61 | 21.61 / 63.94 |
| 4 | 32 | 4096 | iouring-poll | 19.04 / 62.34 | 18.88 / 62.37 |

Median server process CPU seconds, before → after (whole lifecycle):

| Workers | Connections | Bytes | Pure Tokio | Tokio owned | Tokio borrowed | Native owned | Native borrowed |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 1 | 64 | 1.62 → 1.61 | 1.77 → 1.80 | 1.84 → 1.80 | 3.35 → 3.35 | 2.11 → 2.09 |
| 1 | 1 | 4096 | 1.60 → 1.60 | 1.82 → 1.77 | 1.81 → 1.76 | 3.14 → 3.00 | 2.50 → 2.53 |
| 1 | 32 | 64 | 2.54 → 2.53 | 2.72 → 2.67 | 2.70 → 2.67 | 2.88 → 2.86 | 2.80 → 2.79 |
| 1 | 32 | 4096 | 2.61 → 2.61 | 2.77 → 2.71 | 2.76 → 2.70 | 2.91 → 2.89 | 2.83 → 2.84 |
| 4 | 32 | 64 | 8.94 → 8.88 | 9.35 → 9.28 | 9.28 → 9.16 | 12.66 → 12.60 | 11.84 → 11.82 |
| 4 | 32 | 4096 | 9.51 → 9.49 | 9.76 → 9.63 | 9.63 → 9.71 | 12.57 → 12.55 | 11.83 → 11.80 |

## Method and provenance

The final comparison uses the same Aiko Ryzen 9 9950X3D, Linux 7.2.6, Rust 1.99.0, affinity, and build settings as the prototype. Six worker/connection/payload configurations each run three times for both builds and all five backends: 180 server/client samples. Each uses one second of warmup and three seconds of measurement, with combinations shuffled within each repeat using seed `20261025`. Server CPU 0 or CPUs 0–3 and client CPUs 4–7 occupy separate physical cores sharing a last-level cache. Timings hold an exclusive host lock; cooperating tests and builds use shared locking.

- Before source: `14c509acc1c8d8775aa802cbd5c84accd6d5fd3e`, constructed from `6690045baf43409466b5418a1c464b5ef9559329` plus the benchmark-only dispatch change in `75fcc9c`.
- Selected source: `938e7e641f8d186991a0f46f211f4ce3d19c929e`.
- Fixed client source: `5736301fded46c4e5f11d5387950ae2ee0807b94`.

Both servers use `cargo build --release --locked -p core-scheduler-echo-benchmark` with `CARGO_PROFILE_RELEASE_DEBUG=1`: optimization level 3, default code generation and LTO, no `target-cpu=native`, and no coverage instrumentation. The shared target directory is used sequentially; each executable is copied and hashed before another build.

| Executable | SHA-256 |
| --- | --- |
| before | `002163e1acdf63624d03f8f2691e1e9de2b7f1c87f9e7150e2b763f2156fca11` |
| after | `f5f0336a61e7ccfbf6db35b1ac10cf04845137465ef4250c0420691b1173908e` |
| client | `db6c914ab75011477ec6232e681046b1e75532cbaa1c81663bac984591675cf3` |

These closed-loop loopback measurements include client scheduling and kernel networking, and do not establish physical-network or connection-churn performance. Every Core operation passes `Cancellation::never()` and reuses per-connection storage. Ratios are ratios of medians, not confidence intervals; per-run latency percentiles are summarized separately, not pooled. CPU totals include warmup and shutdown. The [methodology](readme.md) explains the limits.

Reproduce with the [serialized runner](results/socket-performance/run.py), choosing a fresh output directory:

```sh
python3 benchmarks/echo/results/socket-performance/run.py \
  --before /tmp/core-perf-socket-control \
  --after /tmp/core-perf-socket-selected \
  --client /tmp/core-scheduler-profile-baseline/release/core-scheduler-echo-benchmark \
  --before-source 14c509acc1c8d8775aa802cbd5c84accd6d5fd3e \
  --after-source 938e7e641f8d186991a0f46f211f4ce3d19c929e \
  --client-source 5736301fded46c4e5f11d5387950ae2ee0807b94 \
  --repeats 3 \
  --output /tmp/core-scheduler-socket-selected-rerun
```

## Verification and remaining work

Canonical whole-workspace Bake coverage with all targets and features passes 18,273/18,273 measured source regions on Linux and 15,817/15,817 on macOS. Linux automatically excludes one existing invariant-only `unreachable!()` region; macOS excludes none. Linux’s temporary LLVM wrapper excludes only distro compiler sources under `/usr/src/debug/rust/`; no project source paths are exempted. Formatting, Clippy, and documentation checks pass on both systems. GitHub Actions also passes Linux, macOS, Windows, publishing checks, and Rust 1.85 compatibility. Independent adversarial review found no selected-runtime correctness blockers.

Regression tests cover independent read/write wakers, concurrent reserved and fallback waiters, dropped and forgotten reservations, cancellation and registration reuse, runtime destruction, syscall-first lifecycle/error results, capacity/listener-registration races, closure, and intermediate/final runnable retirement.

The [prototype profiles](results/socket-performance/profiling/analysis.md) confirm removal of the linked-list readiness-waiter hotspot; the selected Tokio implementation is unchanged from that profile. A final [three-trace syscall check](results/socket-performance/selected/tokio-syscalls/analysis.md) does not establish duplicate receives as the cause of the remaining adapter gap: receive `EAGAIN` counts were 0, 3, and 16 for pure Tokio, owned adapter, and borrowed adapter, respectively, with no send failures. Ptrace changed scheduling and nearly every receive succeeded immediately, so these traces cannot quantify the uninstrumented cost. Tokio clears cached readable readiness after successful short reads on Linux, whereas the adapter clears it after `WouldBlock`; this is a possible source of extra attempts whose normal frequency remains unmeasured. Task-wrapper samples include inlined application work, so they do not isolate bookkeeping cost. Delaying cancellation-future setup added complexity without a convincing gain and was rejected. Native request allocation, completion channels, shared task bookkeeping, and submission/wait overhead remain candidates for future investigation. The small benchmark establishes no isolated cost for each of them.
