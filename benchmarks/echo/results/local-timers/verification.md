# Verification

## Prototype

The archived final candidate passes these Linux checks, serialized with measurements under `/tmp/core-scheduler-perf.lock`:

```sh
cargo test --locked -p core-scheduler-native -p core-scheduler-iouring --all-features --all-targets
```

- io_uring: 78 unit tests and 180 conformance tests.
- Native: 61 unit tests, 20 general conformance tests, six selector composition tests, and 52 socket tests.

The Native unit count includes six new local-timer tests:

- Registration, repoll, and drop allocate nothing within reserved capacity, with existing cancellation state.
- Real local expiry never fires early and does not start the fallback timer thread.
- A pending sleep moves to another polling thread and replaces its waker.
- Remote drop releases a registration; local cooperative cancellation completes.
- Queue exhaustion reports `WouldBlock`, and closure wakes local sleepers with `BrokenPipe`.
- A waker replacement delivered after expiry or slot reuse still wakes the replacement context.

The timer library's existing suite also passes on Linux with all features: 63 unit tests and four integration tests. Clippy passes for Native and Timers with all targets/features and `-D warnings`.

The prototype also passes the Native tests on macOS through the shared Bake workflow:

```sh
rustup run stable cargo bake --locked test:coverage \
  --package core-scheduler-native --all-targets true --all-features true
```

All tests pass, but the coverage gate fails at **2,015/2,040 measured source regions**. The 25 gaps include closure racing remote command admission, pending-sleep closure/registration branches, owner mismatch, inbox retry/empty paths, and some generated test regions. The paused Clock conformance fixture still exercises the original shared queue; it does not establish full conformance of the experimental local implementation. Windows has not been tested for this prototype.

This is research evidence, not a merge-ready production change. In particular, unit tests and benchmarks do not establish every migration, shutdown, cancellation-reclamation, or capacity-policy case.

## Restored production implementation

After archiving the candidate, the main local workspace restores all production source files to the baseline. The same shared Bake command passes **1,335/1,335 measured Native source regions**. Production behavior and public capabilities are unchanged.

The separate empty-check follow-up also passes all 61 Native unit tests; it has no measured performance justification for adoption.

The archived patch passes `git apply --check` against the recorded source commit. The evidence summarizer validates source/patch/binary identities, all expected sample cells, runner/helper identities, and the frozen echo client before regenerating its tables. No binaries, target directories, profiling recordings, or temporary build files are retained in this evidence directory.
