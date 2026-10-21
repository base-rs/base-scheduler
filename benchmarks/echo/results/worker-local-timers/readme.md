# Worker-local timer evidence

The baseline is `1689f8d6d7014a3dcd983b5e88036dc40aa4f527`. The candidate's runtime implementation is `eb0954c7afb566d92173124183f2a860846548fb`; the subsequent report commit adds evidence without changing runtime code. Measurements were collected on Aiko with the standard optimized Cargo profile, no compiler or profile overrides, and `/tmp/core-scheduler-perf.lock` held exclusively. `build.json` records the compiler, build commands, binary hashes, and original cache locations. The candidate was built from an uncommitted patch on the baseline; `eb0954c` commits that implementation. Documentation and test-only additions made afterward do not change the benchmark's runtime code.

- `measurements.json` preserves all 72 echo samples, 18 timer samples, measurement settings, and build metadata.
- `host.json` records the CPU topology and kernel. CPUs 0–7 are distinct physical cores in the same last-level cache.
- `measure.py` is the exact measurement runner, including the randomized order and process CPU accounting. Its SHA-256 is recorded in the raw results.
- `timers.rs` is the identical periodic-timer diagnostic compiled into both variants. It lives outside Cargo targets here; copy it into the private benchmark package to reproduce it.
- `build.py` rebuilds both variants into a separate cache directory and produces the `build.json` consumed by the measurement runner. It refuses a dirty existing worktree except for its own diagnostic source.

On a Linux host with working io_uring, the recorded Rust toolchain, `taskset`, and available physical CPUs 0–7, run from a clone containing both commits:

```sh
python3 benchmarks/echo/results/worker-local-timers/build.py
python3 benchmarks/echo/results/worker-local-timers/measure.py
```

The scripts use `~/Developer/core-rs/core-scheduler-worker-timers-baseline` and `~/Developer/core-rs/core-scheduler-worker-timers-integrated` as worktrees and `~/.cache/core-scheduler-worker-timers-production` for binaries and new results. Preserve or move existing experiment worktrees/cache files first if they conflict; the build script does not reset them. The runner keeps the baseline client fixed across every server sample. It temporarily sets its own inherited timer slack to 1 ns with `prctl`; it does not change a machine-wide policy. Every sample uses a fresh server process. Echo measures one second of warmup and two measured seconds; timers measure one second of warmup and three measured seconds. CPU covers each timer process's whole lifecycle.

The build command is `cargo build --release --locked -p core-scheduler-echo-benchmark --bins`. No extra compiler flags, profile settings, or LTO changes are used. CPU affinity and loopback conditions are part of these results; adjust and record the measurement runner for a different topology instead of treating arbitrary CPU numbers as equivalent physical cores.
