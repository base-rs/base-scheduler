# Worker-owned timer evidence

The [report](../../local-timers-research.md) describes the design and conclusions. The [candidate patch](candidate.patch) is archived and **not applied** to production source.

This is the final revision of the experiment. An earlier screen helped identify the unnecessary empty-queue clock read and waker replacement races; its intermediate results are excluded from these tables. Final binaries were rebuilt with the baseline on unmodified source and the candidate on the archived patch.

| File | Contents |
| --- | --- |
| [build.json](build.json) | Source commit, compiler, patch hash, optimized binary hashes and build command. |
| [provenance.json](provenance.json) | Verified source patch, harness and executable identities, CPU topology and compiler override presence. |
| [timers.json](timers.json) | 48 randomized periodic-timer samples, with matched 1 ns timer slack. |
| [echo.json](echo.json) | 54 randomized echo samples without application timers, including unchanged pure Tokio controls. |
| [mixed-build.json](mixed-build.json) | Diagnostic mixed-load server source hash and optimized binary hashes. |
| [mixed.json](mixed.json) | 24 randomized echo samples with 32 concurrent 1 ms timers. |
| [mixed-stderr](mixed-stderr) | Corresponding timer histograms from each mixed-load server. |
| [stat/results.json](stat/results.json) | Eight separate instrumented timer runs; adjacent files contain whole-process counters. |
| [empty-fast.patch](empty-fast.patch), [empty-fast.json](empty-fast.json) | Separate early-empty-check prototype and 24-sample follow-up; no reliable performance win. |
| [tables.md](tables.md) | Regenerated measurement tables. |
| [summary.json](summary.json) | Numerical aggregates without presentation rounding. |
| [verification.md](verification.md) | Test results, prototype coverage gaps, and restored production coverage. |
| [scripts](scripts) | Exact build, harness, measurement, and mixed-load scripts. |

Validate identities, complete sample matrices, and regenerate the tables:

```sh
python3 benchmarks/echo/results/local-timers/summarize.py
```

## Measurement method

All remote builds, tests, measurements, and profiling acquire `/tmp/core-scheduler-perf.lock` exclusively. They use an isolated worktree at `~/Developer/core-rs/core-scheduler-local-timers` on Aiko. Server/timer worker threads run on CPU 0 or CPUs 0–3; the fixed echo client uses CPUs 4–7. Binaries, target directories, logs and scratch files remain in research caches, outside the evidence directory.

The source baseline is `0d2c4f1c3fa0c2e502531880dab0acba18f097c0`. The candidate adds only `candidate.patch`; both contain the identical diagnostic `timers.rs`. Builds use `cargo build --release --locked -p core-scheduler-echo-benchmark --bins`, adding `--features core-scheduler-native/worker-timers` for the candidate. `build.json` records Rust 1.99.0 / LLVM 23.1.1 and the executable hashes. The initial build screen, which compiled its baseline with the experimental feature disabled on patched source, is not used here: the final baseline is built before applying the patch.

Periodic timers use the same harness as the earlier shared-worker experiment: one second of warmup, three measured seconds, fresh deadlines relative to the previous iteration's completion, per-task HDR histograms, and an assertion against early expiry. The reported rate depends on the requested delay and lateness. It is neither maximum timer throughput nor a fixed-arrival-rate test. CPU time and thread/context-switch observations cover the whole process lifecycle, including warmup, initialization and shutdown; CPU totals are not divided by measured-only expiry counts.

Both timer variants inherit 1 ns timer slack. The timer runner sets `PR_SET_TIMERSLACK` and verifies its own setting before spawning workloads. This controls for the approximately 50 µs coalescing allowance that dominated the first shared-worker latency comparison. It changes no host-wide setting. Counter runs also set 1 ns slack and execute separately from throughput runs.

Echo uses persistent IPv4 loopback connections, one outstanding round trip per connection, `TCP_NODELAY`, reused 64 KiB server buffers, and 64-byte payloads. Client SHA256 is `db6c914ab75011477ec6232e681046b1e75532cbaa1c81663bac984591675cf3`. Each sample has one second warmup and three measured seconds. Pure Tokio is unchanged by the timer feature and controls for variation in the test environment. Echo without application timers retains the ordinary inherited slack; timer policy is not exercised by its request path.

The mixed harness adds 32 periodic 1 ms timer tasks alongside the unchanged Native echo tasks. Both mixed variants inherit 1 ns slack. Histograms record deadlines after the server's first second until echo completes. This approximates the client's three-second measurement window, with small startup/teardown differences; it does not provide exact synchronized per-request latency or a fixed timer arrival rate. The server stops the timer tasks, awaits them, reports their histogram on stderr, then shuts down. `mixed-server.rs` is the exact generated source. These binaries and workloads are distinct from the no-timer echo screen.

All uninstrumented tables use medians of three randomized repetitions. Counter tables use medians of two separate runs. These short experiments can reveal material changes; they do not establish sub-percent equivalence or broad application performance.

## Reproducing

The archived scripts retain the exact cache paths used during the experiment. Create a dedicated worktree at `~/Developer/core-rs/core-scheduler-local-timers` on the source commit. Do not point `build.py` at a checkout containing other work: it resets that dedicated worktree before building the baseline.

Copy `candidate.patch` to `~/.cache/local-timers-final.patch`, and `scripts/timers.rs` to `~/.cache/core-scheduler-worker-timers/timers.rs`. Copy the generic `run.py` and adjacent `sample-runner.py` to `~/.cache/core-scheduler-single-worker-research/`, and retain the recorded fixed client at `~/.cache/core-scheduler-owner-wake-final/client`.

Run `scripts/build.py` first. It builds and freezes the unmodified baseline, applies the candidate, builds and freezes the feature-enabled version, and runs Linux Native unit tests. Outputs go to `~/.cache/core-scheduler-local-timers-final/`.

Run `scripts/run-timers.py`, `scripts/run-echo.py`, and `scripts/stat-timers.py`. Output subdirectories must not already exist. Counter collection requires the host's existing passwordless `sudo -n perf` configuration; `setpriv` runs the measured executable as the original user.

Run `scripts/build-mixed.py` and then `scripts/run-mixed.py` for mixed-load measurements. The build script temporarily substitutes the diagnostic server source in the dedicated checkout and toggles the timer patch to freeze both variants; it restores the ordinary server source afterward. The timer patch remains applied there. The main local workspace retains only this research archive.

The optional `scripts/empty-fast.py` waits for all eight primary counter samples, builds and unit-tests an additional early-empty-check patch, then runs the single-connection follow-up. It restores the original candidate source afterward. These results do not replace the primary candidate measurements. `scripts/provenance.py` verifies the restored candidate diff and records the exact workload hashes.
