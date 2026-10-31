# Lazy cancellation setup screening

Candidate: baseline source `4a061fbc4355738f39f5eb6ea1db1520db6a753e` plus the exact uncommitted edits retained in `source.patch`. The candidate moves cancellation racing into the socket readiness helper, after the initial direct syscall returns WouldBlock. The caller still checks cancellation and scheduler/socket validity before the syscall. Registration, reserved waiter ownership, concurrent fallback, operation-first race preference, and lifecycle/error behavior remain unchanged.

Build: `CARGO_PROFILE_RELEASE_DEBUG=1 cargo build --release --locked -p core-scheduler-echo-benchmark`; shared host lock for the build; exclusive host lock for the screen. Frozen binary hashes are in `aiko.json`.

Screen: three randomized repetitions, one worker, 32 connections, 64-byte messages, one-second warmup and three-second measurement. Server CPU 0; fixed client CPUs 4–7. Both source variants use the same benchmark dispatch implementation.

| Backend | Baseline median req/s | Candidate median req/s | Change |
| --- | ---: | ---: | ---: |
| tokio-adapter | 618,989 | 626,032 | +1.14% |
| tokio-adapter-poll | 624,154 | 627,897 | +0.60% |

Pure Tokio control median: 701,652 req/s.

Decision: no convincing improvement sufficient to address the remaining adapter gap. Candidate changes were restored from the worktree after preserving the exact patch and executable. No tests were run for this rejected candidate. Do not infer that all cancellation overhead is negligible: this experiment only delays listener/future setup until waiting is required; pending receives still race cancellation.

The larger remaining gap is not resolved by this small state-machine refactoring. The exact initial WouldBlock/readiness retry ordering remains intact, so this screen does not isolate duplicate syscall overhead.
