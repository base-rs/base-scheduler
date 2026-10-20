# Borrowed native performance investigation

Base: merged `427be5d2ef6c610a5d5de5b10570c938b65d6c2d`, isolated `perf/native-next` worktree. No candidate is recommended or retained in source.

All builds are optimized default release builds with the same fixed owner-wake-final client and baseline server. Initial screens have three randomized baseline/candidate repetitions per workload, one-second warmup and three-second measured window; server CPU0 or CPUs0-3, client CPUs4-7. Timings and profiles held the exclusive shared performance lock; builds held its shared mode. Raw sample files, runner sources, candidate binaries, exact patches and hashes are preserved here.

| Candidate | Workers/connections | Baseline median req/s | Candidate median req/s | Change |
|---|---:|---:|---:|---:|
| completion | 1/32 | 566,143 | 572,647 | +1.15% |
| completion | 4/128 | 1,959,167 | 1,987,163 | +1.43% |
| route | 1/1 | 229,742 | 225,198 | -1.98% |
| route | 1/32 | 564,980 | 566,948 | +0.35% |
| worker | 1/1 | 228,476 | 220,037 | -3.69% |
| worker | 1/32 | 566,309 | 567,577 | +0.22% |

Completion replaced only readiness futures-channel oneshot replies with a safe Mutex/owned-waker state and generalized private submit's result future. Its ~80 lines are unjustified for these small improvements. Route returned the only controller directly for a single-ring selector; gain is negligible and serial ranges overlap. Worker directly woke the only worker, retaining its SeqCst sleeping exchange/driver notification, and skipped single-worker peer fan-out; it gave negligible concurrent gain and a concerning serial regression. All three are rejected; these short screens do not establish final capacity differences at one-percent precision.

The baseline user-cycle profiles show native work dominated by wait_ready (~10%), readiness oneshot bookkeeping (~20%), capacity acquire (~6%), route (~5%), and wake_idle (~4%). Native lifecycle server user CPU was0.82s versus Tokio0.38s, with system CPU3.46/3.48s during their separate six-second profiled workloads. Profiling throughput is not included in the timing table. Profiles confirm per-WouldBlock request admission/allocation/reply costs, but simplifying the channel removed little measured wall cost. Persistent/multishot readiness could eliminate per-wait kernel registration and request bookkeeping, but needs a resource-lifetime and cancellation design; no such unmeasured architecture change was attempted.

See `summary.json` for ranges, phase directories for raw results, `baseline-profile` for perf.data/self/callgraph reports, and `provenance.json` for hashes/build settings. CPU/context-switch fields in samples cover server lifecycle, while request rate covers only the measurement window.
