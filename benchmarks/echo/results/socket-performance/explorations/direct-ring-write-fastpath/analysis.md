# Native direct-notification and owned-send candidate

Selected: direct io_uring idle waits, remote eventfd notification with owner TLS permits, drain-before-rearm, full CQ publication, original-ring owned retries, combined submit/wait with empty-submit elision, payload/cancellation-first SQ filling, synchronous nonblocking owned scalar send before allocating completion-path storage. Read-first is rejected.

## Concurrent throughput

Three randomized repeats per cell; median requests/s. Plain optimized release builds, fixed client, server CPUs 0 or 0–3 and client CPUs 4–7. One-second warmup and three-second measurement. Exclusive benchmark lock.

| Workers | Connections | Bytes | Backend | Original | Candidate | Change |
|---:|---:|---:|---|---:|---:|---:|
| 1 | 32 | 64 | iouring | 532,161 | 626,804 | +17.78% |
| 1 | 32 | 64 | iouring-poll | 561,774 | 565,945 | +0.74% |
| 1 | 32 | 4096 | iouring | 479,236 | 559,441 | +16.74% |
| 1 | 32 | 4096 | iouring-poll | 506,987 | 508,186 | +0.24% |
| 4 | 128 | 64 | iouring | 1,782,400 | 1,976,510 | +10.89% |
| 4 | 128 | 64 | iouring-poll | 1,955,956 | 1,967,051 | +0.57% |
| 4 | 128 | 4096 | iouring | 1,600,101 | 1,747,017 | +9.18% |
| 4 | 128 | 4096 | iouring-poll | 1,746,562 | 1,745,486 | -0.06% |

Owned concurrent throughput improves 9–18%; borrowed concurrent throughput is neutral within about 1%.

**Data recovery:** the subsequent serial runner accidentally reused the matrix aggregate path. All 48 sample throughputs (rounded to nearest request/s), exact voluntary switches, and full-precision medians/deltas survive in the completed `run-matrix.log`. `matrix-recovered.json` documents reconstruction and missing detailed RTT/CPU fields; these fields have not been invented.

## Serial throughput

Five randomized repetitions per binary/backend/payload, one worker and one connection. Full per-sample metadata retained in `serial-five/aiko.json`.

| Bytes | Backend | Original | Candidate | Change |
|---:|---|---:|---:|---:|
| 64 | iouring | 237,522 | 230,564 | -2.93% |
| 64 | iouring-poll | 181,897 | 223,304 | +22.76% |
| 4096 | iouring | 186,810 | 232,078 | +24.23% |
| 4096 | iouring-poll | 203,502 | 233,748 | +14.86% |

Owned 64B serial throughput remains about 2.9% below original; the other serial cases improve 14.9–24.2%. Final-source timings will be rerun after integration of the cancellation-queue guard.

## Idle and notification mechanism

Frozen plain candidate: zero CPU ticks across owned/borrowed × one/four workers in two-second idle windows. Native ring workers sleep in `io_cqring_wait`; see `idle/idle.json`.

A separately instrumented binary counts actual eventfd write attempts/success by exact controller TLS ownership; it also counts owner wakeups that avoided a syscall. Instrumented throughput is excluded from all timing tables.

| Backend | Workers/connections | Owner writes | Remote successful writes | Avoided owner wakes |
|---|---|---:|---:|---:|
| iouring | 1/1 | 0 | 2 | 39 |
| iouring-poll | 1/1 | 0 | 2 | 12 |
| iouring | 4/128 | 0 | 38,052 | 14,139 |
| iouring-poll | 4/128 | 0 | 26,201 | 6,142 |

See `counted/samples/aiko.json` for all thread counts and binary/instrumentation hashes. This mechanism proof uses the selected candidate before the final cancellation-queue idle guard; an exact-production fresh timing/idle run will be stored separately under `../core-scheduler-owner-wake-final`.

## Reproducibility

`provenance.json` records source/runner/binary SHA-256s, commit, machine, CPU placement, rustc/cargo versions, and absence of release-profile overrides. Full source is `source.patch` against `63da7551abbd777ac0f746ae2530eb7968009fe4`. Timing binary SHA-256: `103d4f6f0a263312edf55d48a97341c03c8f8ec4106940111a0752942629855f`.

## Rejected receive fastpath

Read+write passed all 180 conformance tests and idle checks but regressed serial owned throughput by 13.1% against control. Only send-first is selected. Read-first typically adds an unsuccessful syscall before ring submission. Evidence is retained under `../core-scheduler-direct-notification-scalar-fastpath`.
