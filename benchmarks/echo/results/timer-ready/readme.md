# Stealable timer delivery experiment

This extends the [worker-owned timer experiment](../local-timers/readme.md). All variants use its archived [candidate.patch](../local-timers/candidate.patch), applied to source revision `64eb1548e1371b067f0e7c8b85d5a37c2f745768`. These are research patches, not enabled production features.

| Label | Additional patch | Native feature | Behavior |
| --- | --- | --- | --- |
| `immediate` | None | `worker-timers` | Pop each due waker and invoke it immediately; the resulting runnable enters a stealable ready queue. |
| `ready` | [ready.patch](ready.patch) | `ready-timers` | Queue each due waker as a stealable `Ready::Timer` action; its eventual wake queues the task. |
| `batch` | [batch.patch](batch.patch) | `worker-timers` | Collect due wakers under one timer borrow into reusable storage, then wake them immediately. |
| `ready-batch` | [ready-batch.patch](ready-batch.patch) | `ready-timers` | Collect due wakers, account for and publish the whole ready batch with one worker borrow and idle notification. |
| `ready-local` | [ready-local.patch](ready-local.patch) | `ready-timer-local` | Add local routing during timer delivery, allowing a stealing worker to keep synchronously awakened tasks. |

Each additional patch applies directly after the original candidate; do not stack these patches. Both ready queue generations remain stealable. Queued timer actions count as outstanding until their waker callback returns, including during scheduler shutdown. Cancellation/drop may release a registration after expiry was collected; that does not retract an already queued wake.

The queue enum is 24 bytes on the measured 64-bit target, compared with the original 8-byte `Runnable`. Batch variants reserve a reusable `Vec<Waker>` matching the timer queue's actual rounded capacity: another 16 bytes per reserved timer slot per worker. No per-expiry box or task is allocated by these changes. Queue growth remains governed by the existing crossbeam queues.

## Evidence and validation

- `*-build.json`: source, compiler, release build command, file hashes, and frozen executable hashes.
- `source-verification.json`: independently reconstructed source hashes for all five variants and patch hashes.
- `provenance.json`: final verification that frozen executable hashes still match, plus machine/kernel details.
- `timers/`: 45 samples comparing immediate, ready, and batch across five workloads.
- `optimized-timers/`: 36 fresh samples comparing immediate, ready-batch, and ready-local across four workloads.
- `echo/`, `mixed/`: 24 and 18 samples for the first comparison.
- `optimized-echo/`, `optimized-mixed/`: 24 and 18 samples for the optimized comparison.
- `stat/`: six separate instrumented periodic-timer samples.
- `scripts/`: harnesses and commands used on Aiko. Builds, tests, and measurements acquire the same exclusive `/tmp/core-scheduler-perf.lock`.

Regenerate and validate all 171 samples, executable identities, harness hashes, and mixed-workload histogram records:

```sh
python3 benchmarks/echo/results/timer-ready/summarize.py
```

Results are generated into [tables.md](tables.md) and [summary.json](summary.json). The interpretation is in the [research report](../../timer-ready-research.md).

## Method

All executables were rebuilt with `cargo build --release --locked`, using the same Rust compiler, dependencies, harness, and feature-appropriate scheduler sources. There were no `RUST*` or `CARGO_PROFILE*` environment overrides. Builds and measurements run serially under the performance lock. Executables are frozen before measurement; no profiling runs overlap uninstrumented samples.

Three repetitions per cell are randomized within each screen. Server/timer processes use CPUs `0..workers-1`; the independent echo client uses CPUs `4–7`. All runners set their own Linux timer slack to 1 ns, inherited by children and threads. Timer runners verify this through `/proc/self/timerslack_ns`. The echo runner is launched through the same explicit `prctl` setting. Host policy is unchanged.

Periodic timer samples use one second of warmup and three measured seconds. Relative timers request a new deadline after completion. Aligned timers request the next common 1 ms boundary, skipping missed boundaries; their rate is bounded by the configured cadence, not a maximum-throughput measurement. `work=10000` adds a deterministic, black-boxed integer recurrence after each wake, saturating the four workers. Lateness is recorded before that work. CPU is whole-process user plus system time, including warmup, startup, and shutdown; histogram counts include only the measured window.

Echo uses borrowed `_poll` socket operations, 64-byte persistent IPv4 loopback messages, TCP_NODELAY, and one outstanding roundtrip per connection. Mixed runs add 32 independent relative 1 ms timers, with a histogram starting after the server's first second and ending after echo finishes. This approximately aligns the timer and client measurement windows. Tokio in no-timer screens is a noise control, not a fresh Tokio performance investigation.

The initial mixed build copied a source file with an older preserved mtime, which let Cargo reuse an ordinary echo executable. Identical executable hashes exposed this before any mixed samples ran. [fix-mixed.py](scripts/fix-mixed.py) rebuilt those three mixed executables using fresh source mtimes and asserted distinct hashes. Later variants use the corrected build script. Embedded build records in earlier timer/echo datasets retain the earlier unused mixed hashes; their actually executed timer/ordinary binaries match the canonical `*-build.json` records. The summarizer validates the executable used by each dataset.

## Tests and limits

All four additional variants pass Linux Native unit, conformance, selector, and socket tests, plus io_uring's 78 unit tests, 180 conformance tests, and documentation example. Native unit counts are 64 (`ready`), 62 (`batch`), 66 (`ready-batch`), and 68 (`ready-local`); each also passes 20 conformance, 6 selector, and 52 socket tests. Commands are preserved in `scripts/build.py`.

Added regressions cover stealing timer delivery from both generations, queued wake delivery after the sleep is dropped, outstanding accounting through callback completion, empty publication batches, reusable allocation-free batch storage, callbacks outside timer borrows, routing a stolen timer's task locally, and restoring routing after a panicking callback. The implementation agents also ran macOS stable tests and Clippy for their variants.

These tests do not establish complete source-region coverage or complete worker-local clock conformance. The original worker-local prototype's external-thread fallback, capacity policy, paused-clock fixture, and other adoption work still apply. No experimental patch is applied to production source.
