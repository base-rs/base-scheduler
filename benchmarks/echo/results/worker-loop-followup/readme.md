# Worker-loop follow-up evidence

Run `python3 summarize.py` from any directory to validate and summarize 136 samples: 72 timer primitive runs, 36 scheduler timer runs, and 28 echo control/profile runs. The [research report](../../worker-loop-followup.md) explains conclusions and limits.

- `timer-build.json` identifies sources, release commands, patches, compiler, and binary hashes for four timer variants.
- `timer-micro.json` contains nine repetitions of each primitive and variant. `scripts/queue_perf.rs` is the measured primitive harness.
- `timers/results.json` contains the complete timer scheduler matrix. The unchanged harness is [archived with timer-ready research](../timer-ready/scripts/timers.rs); the original local implementation is [archived with local-timer research](../local-timers/candidate.patch). The summarizer checks these source hashes.
- `timer-loop-profile.json` identifies frozen production/local binaries, client, scripts, and every control, `perf stat`, and cycle-profile sample. The original local build is based on `0d2c4f1`, whose production behavior matches `64eb154` used for the new builds.
- `timer-loop/*/stat` contains full perf counters. `timer-loop/*/perf` contains complete self-symbol and callgraph text reports compressed with gzip. `profile-text-normalization.json` records original hashes, removal of trailing whitespace, and compressed/uncompressed archive hashes. No symbol or report line was removed. Binary perf recordings remain in Aiko's scratch cache.
- `timer-flush.patch` is the production change. `local-pop.patch` is the additional local-queue optimization. `local-pressure.patch` is a separately tested local-prototype capacity fix, absent from measured binaries.
- `timer-feature-verification.json` records the final macOS and Linux driver-only and sharded-only Bake gates, exact commands, and log hashes. It also preserves a first Linux coverage failure caused by reusing a target with unused sharded dependency instrumentation; both features pass with independent `CARGO_LLVM_COV_TARGET_DIR` values, as used by CI.
- `scripts` retains build, timing, profiling, and timer verification programs. Their absolute Aiko cache/worktree paths describe the original run environment; rebuild in equivalent isolated worktrees, placing referenced patches and harnesses at those paths or adapting paths explicitly.

No executables, target directories, or binary perf recordings are stored here. The empty-injector experiments and confirmation are in [worker-hotpath](../worker-hotpath/readme.md).

## Combined submission and waiting

`python3 summarize-driver.py` validates a separate 285-sample driver study: an initial 45-sample screen, a 45-sample small/large-message confirmation, a 25-sample concurrent confirmation, ten diagnostic counter samples, a 75-sample combined-wait/collection matrix with ten additional counter samples, and a 75-sample minimal post-wait collection matrix. These samples use the two small production queue improvements in both baseline and candidate. Their unchanged ordinary-read Tokio control is not a measurement of interface dispatch overhead or an equalized receive policy.

- `common-hotpath.patch` is the shared baseline improvement; `driver-turn-prepared.patch` is the first driver candidate. `driver-build.json` records exact sources, compiler, build commands, patch hashes, and binaries.
- `driver-matrix.json`, `driver-confirm.json`, and `driver-concurrent.json` retain every uninstrumented sample, random order, load, client/helper/runner identity, and matching control.
- `driver-stat.json` and `driver-stat/*/*.stat` retain separate syscall and hardware counter runs with no warmup. Counter ratios use the completed exchange count from that same diagnostic sample. Instrumented throughput is not a capacity comparison.
- `scripts/build-driver-turn.py`, `scripts/run-driver.py`, `scripts/stat-driver.py`, and `scripts/verify-driver.py` reproduce the build, timing, counter, and Bake coverage commands on Aiko. All acquire the shared exclusive lock.
- `scripts/prepared-pointer-miri.rs` retains three focused aliasing models used while reviewing early buffer preparation. To rerun, place it in a temporary Rust library and use `cargo +nightly miri test`. These models are supporting evidence, not a complete proof: integer pointer storage uses exposed provenance and Miri emits the corresponding warning. Actual request ownership, original-completion retirement, and cancellation also remain covered by the Linux runtime tests.

The first candidate combines idle submission and waiting but returns before collecting the newly available CQEs. `driver-wait-drain.patch` is a separate incremental experiment which collects that snapshot before returning. Its build and timing scripts are retained separately so it cannot silently replace the first candidate's source or results.

- `driver-drain-build.json`, `driver-drain-matrix.json`, and `driver-drain-stat.json` identify this combined variant and its 85 samples; `driver-drain-stat/` retains the counters. Its io_uring coverage stopped at 2,138/2,139 regions, as recorded in `linux-verification.json`.
- `wait-drain-only.patch` isolates completion collection after waiting, without combining submission/waiting, changing the worker order, or changing request representation. `wait-drain-only-build.json` and `wait-drain-only-matrix.json` identify its build and 75 samples. `scripts/build-wait-drain-only.py` records the transition from the larger prototype and successful Linux coverage/Clippy verification. The build script describes that original transition; do not rerun its reverse-patch operations against an already transitioned checkout.
- `linux-verification.json` records the timer and driver gates, including the archived prototype shortfall and the minimal candidate's successful Native 1,337/1,337 and io_uring 2,131/2,131 region gates. One invariant-only io_uring region is excluded by the shared gate.

Both combined-wait prototypes and the minimal completion-collection prototype remain archived. Only the common admission-queue and timer-flush guards, plus the idle-wake regression test, are applied in the primary source tree. After measurements, the isolated Aiko checkout was restored to that common patch too; frozen experimental executables remain in the scratch cache.
