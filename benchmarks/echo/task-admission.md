# Round-robin Native task admission

New Native tasks previously entered a shared unassigned queue when spawned externally, or inherited the submitting worker's local queue. Idle stealing distributed runnable work, but did not necessarily distribute connections before they acquired persistent ring state. Stealing the consumer later does not move an active multishot receive or its kernel payload-copy work.

Task admission now selects a preferred worker round-robin under its existing lock. Nested spawns participate in the same sequence. The initial runnable uses the selected worker's local queue or remote inbox; subsequent wakes follow the last polling worker. The shared unassigned queue is removed. There is no new allocation, synchronization primitive, or per-operation load counter.

This is best-effort initial placement, not task pinning or a guarantee of equal CPU, connection, or I/O load. Idle workers can still steal initial runnables. A root passed to `block_on` continues to run on the calling thread.

## Echo comparison

Seven randomized paired repetitions compare optimized baseline and candidate binaries with an unchanged baseline client. Servers use `iouring-monitor`, one or four workers, one or 32 persistent connections, and 64-byte or 4 KiB payloads. Each sample has 0.5 seconds of warmup and 1.5 seconds of measurement. Server processes are restricted to the first `workers` CPUs in 0–3; clients use CPUs 4–7 with four Tokio workers. These are separate physical cores sharing one L3 domain on Aiko. Threads are not individually pinned within each process's CPU set.

Throughput is echo round trips per second. Changes are medians of matched per-repeat ratios, not ratios of the independent throughput medians. Ranges are observed paired ranges, not confidence intervals. Positive p99 changes mean higher latency.

| Workers | Connections | Bytes | Baseline round trips/s | Candidate round trips/s | Paired throughput change | Observed range | Paired p99 change |
| --- | --- | --- | ---: | ---: | ---: | --- | ---: |
| 1 | 1 | 64 | 267,324 | 267,331 | −0.1% | −3.2% to +2.2% | +0.3% |
| 1 | 1 | 4096 | 232,045 | 234,063 | +0.4% | −1.8% to +2.0% | 0.0% |
| 1 | 32 | 64 | 610,668 | 617,897 | +1.1% | −0.8% to +2.7% | −1.4% |
| 1 | 32 | 4096 | 547,337 | 557,073 | +1.8% | +0.7% to +2.3% | −1.8% |
| 4 | 1 | 64 | 262,991 | 265,625 | +1.0% | −0.3% to +2.3% | −1.1% |
| 4 | 1 | 4096 | 233,011 | 233,790 | +0.2% | −1.8% to +2.1% | +4.1% |
| 4 | 32 | 64 | 1,704,753 | 1,730,710 | 0.0% | −1.7% to +4.3% | −0.5% |
| 4 | 32 | 4096 | 1,589,795 | 1,571,419 | −0.6% | −2.4% to +3.5% | +0.5% |

The echo controls do not show a clear throughput regression. They also do not establish a broad latency improvement: the four-worker, single-connection 4 KiB case has a +4.1% median p99 change despite similar throughput. These short closed-loop samples are not an open-loop tail-latency study or a connection-churn benchmark.

## Persistent receiver placement

The motivating pooled-receive experiment uses a separate checkout containing the worker-owned buffer prototype. Both variants have identical diagnostic instrumentation and continue to choose receiver rings from the current worker; the diagnostic receiver round-robin override is disabled. Only Native task admission and removal of the unassigned queue differ between the two builds.

Seven randomized paired repetitions use eight connections, bursts of eight 64 KiB logical payloads, 30,000 measured bursts per connection after 1,000 warmup bursts, and 64 total payload slots. Each sample receives 125,829,120,000 measured bytes. Affinity is CPUs 2–15, matching the earlier receive investigation and spanning both L3 domains. Individual threads are not pinned. Ordinary monitor pools and equal-total-budget worker provided-buffer pools are compared independently at each worker count.

| Workers | Receive path | Baseline payloads/s | Candidate payloads/s | Paired throughput change | Observed range |
| --- | --- | ---: | ---: | ---: | --- |
| 1 | Monitor | 561,414 | 555,864 | −0.8% | −4.4% to +4.8% |
| 1 | Provided multishot | 525,282 | 527,385 | +0.2% | −1.0% to +0.8% |
| 2 | Monitor | 1,039,687 | 1,031,836 | −0.4% | −1.7% to +0.8% |
| 2 | Provided multishot | 890,942 | 900,082 | +0.5% | −9.2% to +33.2% |
| 4 | Monitor | 1,100,664 | 1,100,170 | −0.4% | −9.1% to +18.2% |
| 4 | Provided multishot | 1,050,181 | 1,164,070 | −0.1% | −11.1% to +27.2% |

Placement improves without becoming deterministic:

- Two workers: baseline has four 4/4 assignments and three 6/2 assignments. Candidate has six 4/4 assignments and one 5/3 assignment.
- Four workers: baseline has one 2/2/2/2 assignment, one 3/3/1/1 assignment, and five permutations of 3/2/2/1. Candidate has four 2/2/2/2 assignments, two permutations of 3/2/2/1, and one 3/1/3/1 assignment.

Admission therefore reduces the observed frequency of uneven placement but does not prevent stealing or migration before receiver construction. In the two-worker samples, baseline 4/4 runs have a median of about 897k payloads/s, while its 6/2 runs have a median of about 678k. Candidate 4/4 runs have a median of about 900k; its single 5/3 run achieves about 816k. This is consistent with placement affecting throughput rather than a faster individual receive implementation.

The median paired throughput changes for provided receives are small, and the ranges remain broad. A separate aggregate describes the cost of the uneven runs: summing all seven equal-work samples gives 16.833 baseline versus 15.146 candidate measured seconds with two workers, a 10.0% time reduction. Four-worker totals are 12.960 versus 12.255 seconds, a 5.4% reduction. Equivalently, aggregate throughput rises 11.1% and 5.8%. This aggregate is total identical work divided by total measured time, not the median paired change or evidence of a guaranteed per-run speedup. Each variant transfers the same bytes; timing excludes warmup and setup.

## Interpretation

Round-robin is an admission policy, not an estimate of current workload. Counting live tasks would include idle connections and would not measure their CPU or persistent ring load. A more adaptive policy needs evidence that its estimates improve placement enough to justify their maintenance cost.

Keep ordinary wakes local and retain idle stealing for uneven work and blocked workers. Do not introduce per-receive placement counters or migrate active ring operations as part of this change. The tests include a blocked parent with a full admission round of children, so one child must actually be stolen for progress; direct round-robin dispatch alone cannot satisfy that regression.

## Provenance and reproduction

All 196 samples were collected on Aiko, an AMD Ryzen 9 9950X3D running Linux `7.2.6-arch2-1` and Rust `1.99.0`. Builds, tests, and timings were serialized with `/tmp/core-scheduler-perf.lock`.

- Echo baseline: `6b9f8a3`, the main branch before this change.
- Pooled-receive base: `c461fe8ff281d6d694c390d0f97d0096df730534` plus the preserved `balance.patch` from the [earlier investigation](https://github.com/core-rs/core-scheduler/blob/5e745a22f10f1e5f084c813f8454daf1be1a4d2a/benchmarks/buffers/receive-cost.md). That diagnostic patch is a standalone alternative, not a patch series.
- Both candidates add the same runtime patch recorded in [provenance.json](results/task-admission/provenance.json). It predates an import-order-only formatting adjustment; subsequent Rust changes add tests only.
- Builds use `cargo build --release --locked` with `CARGO_PROFILE_RELEASE_DEBUG=1`, selecting `core-scheduler-echo-benchmark` or `core-scheduler-buffer-benchmark --bin sockets`. Debug symbols do not disable release optimization.
- [echo.jsonl](results/task-admission/echo.jsonl) contains 112 samples, including client/server hashes and latency measurements.
- [pools.jsonl](results/task-admission/pools.jsonl) contains 84 samples, including actual receive counts, original ring ownership, diagnostics, exact commands, and binary hashes. The owner list is meaningful only for provided receivers; monitor rows have no persistent receiver-owner diagnostics.
- [run.py](results/task-admission/run.py) is the exact experiment driver, with seed `20261022`. Its Aiko paths refer to isolated worktrees and preserved binaries; adjust those paths after recreating the builds. It imports the existing echo runner without rebuilding or changing the fixed client.

The data measures loopback transport without HTTP, TLS, retained parser views, heterogeneous connection loads, or real-network constraints. Do not combine these results with other diagnostic datasets into an additive cost breakdown or infer that buffer locality is solved by task placement.

## Validation

All-target, all-feature workspace tests and region coverage pass on macOS at 18,498/18,498 measured regions and Linux at 22,620/22,620, excluding one and two invariant-only unreachable regions respectively. Formatting, workspace Clippy, and API documentation checks pass. Deterministic tests cover sequential and concurrent admission, one and three workers, nested spawns, stable wake placement, and actual stealing from a blocked worker.

Aiko uses its packaged LLVM tools. Its coverage wrapper excludes unavailable external Rust standard-library source paths under `/usr/src/debug/rust/` in addition to the existing coverage exclusions; no project source is excluded by that workaround. Windows and MSRV checks remain part of the pull request's CI matrix.
