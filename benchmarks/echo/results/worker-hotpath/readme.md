# Worker hot-path evidence

Run `python3 benchmarks/echo/results/worker-hotpath/summarize.py` from the repository root to validate the archived samples and regenerate `tables.md` and `summary.json`.

- `echo-results/aiko.json`: 54 samples for the single-worker local-notification shortcut.
- `injector-results/aiko.json`: 54 samples for empty admission-queue checks.
- `injector-confirmation/aiko.json`: 25 fresh single-connection samples, five repetitions for baseline/candidate owned and borrowed paths plus the unchanged baseline Tokio control.
- `build.json` and `build-injector-empty.json`: exact source, compiler, patch, and frozen executable identities.
- `single-local.patch` and `injector-empty.patch`: original patches used to build the measured executables.
- `*-reviewed.patch`: corresponding patches with a stronger idle-driver regression fixture. Their production `worker.rs` changes are identical to the measured patches, which the summarizer verifies. Only the reviewed injector change is applied to production.
- `build.py`, `build-injector.py`, `run.py`, and `sample-runner.py`: exact build and sampling scripts. Paths identify their isolated Aiko worktrees and caches; adapt paths when reproducing in a fresh checkout. Build scripts require the recorded `0d2c4f1` base, whose production and benchmark sources match `64eb154`.

The summary validates every expected backend/variant/workload/repetition cell, sample counts, runner and helper hashes, patch hashes, matching build manifests, executable hashes, and the unchanged production diff between original and reviewed patches. No sample is removed or reconstructed. Binaries, Cargo targets, build logs, and scratch files are excluded from this evidence.

The archive contains 133 validated samples in total. Each main screen uses three randomized repetitions of six cases across three loads. For each of `before` and the candidate, the cases run `iouring`, `iouring-poll`, and `tokio`. Loads are 1 worker/1 connection/64 bytes, 1/32/64, and 4/128/64. Scripts acquire the exclusive `/tmp/core-scheduler-perf.lock` before builds, tests, and measurements.

The confirmation uses the same runner with `--repeats 5 --workload 1,1,64`, four Native cases (baseline and candidate, each owned and borrowed), and a fifth baseline Tokio control. Its manifest is unchanged because the frozen binaries are reused.

Both original variants pass the Linux Native test suite. The single-worker shortcut originally passed Native's macOS region gate at 1,340/1,340 regions. The reviewed injector variant passes at 1,337/1,337 regions, with 56 unit tests, 20 conformance tests, 6 selector tests, 52 socket tests, and 2 doctests; zero regions are excluded. Formatting and all-target Native Clippy with warnings denied pass. Coverage and tests use Rustup stable on macOS; Aiko build manifests record Arch Rust 1.99.0 and LLVM 23.1.1.

The focused regression distinguishes same-thread completion publication from a remote wake during an idle driver turn. Merely observing the executor's idle count can race its post-publication search, where a new task can be found without an interrupt. The reviewed fixture observes entry into the actual blocking driver turn before requiring the remote interrupt. Existing tests cover the queued-before-park wake permit, completion batching, self-wake fairness, external submissions, and local work stealing while an owner blocks.
