# Final native owner-wake benchmark

Production source: `3af77be0f8b8a9303c8680a7f8aab49b1c3d304c`; verified runtime patch SHA-256 `325f061c333381555fc100df4f720cc4a51294c99f90d3acf8e8177da4029535`. The benchmark worktree applies that exact production patch to `63da7551abbd777ac0f746ae2530eb7968009fe4`. The benchmark worktree imports the production source files; additional test and documentation changes from that commit were not imported, as recorded in `source-commit.json`.

Optimized release builds use `cargo build --release --locked -p core-scheduler-echo-benchmark`, with no release-profile or RUSTFLAGS overrides. Original control and final candidate use the same build configuration and fixed client. Each case has three randomized repetitions, one-second warmup and three-second measurement; server CPUs 0 or 0–3 and client CPUs 4–7. All timing/idle runs hold the exclusive benchmark lock. These results compare the final native backend against its original native baseline, not against pure Tokio.

## Concurrent throughput

| Workers | Connections | Bytes | Backend | Original requests/s | Final requests/s | Change |
|---:|---:|---:|---|---:|---:|---:|
| 1 | 32 | 64 | iouring | 535,082 | 629,140 | +17.58% |
| 1 | 32 | 64 | iouring-poll | 565,031 | 565,627 | +0.11% |
| 1 | 32 | 4096 | iouring | 485,437 | 556,065 | +14.55% |
| 1 | 32 | 4096 | iouring-poll | 512,053 | 509,279 | -0.54% |
| 4 | 128 | 64 | iouring | 1,791,794 | 1,975,848 | +10.27% |
| 4 | 128 | 64 | iouring-poll | 1,968,757 | 1,960,252 | -0.43% |
| 4 | 128 | 4096 | iouring | 1,585,623 | 1,746,889 | +10.17% |
| 4 | 128 | 4096 | iouring-poll | 1,746,540 | 1,743,991 | -0.15% |

## Serial 64-byte throughput

| Workers | Connections | Bytes | Backend | Original requests/s | Final requests/s | Change |
|---:|---:|---:|---|---:|---:|---:|
| 1 | 1 | 64 | iouring | 234,402 | 234,387 | -0.01% |
| 1 | 1 | 64 | iouring-poll | 184,665 | 230,231 | +24.67% |

## Idle CPU

All eight original/final × owned/borrowed × one/four-worker samples recorded zero CPU ticks over their two-second idle windows. Final ring workers sleep in `io_cqring_wait`. `idle/idle.json` retains per-thread wait channels and binary hashes.

## Separate notification mechanism proof

The immediate predecessor candidate was measured with a separately instrumented binary before the final cancellation-queue idle guard and syscall test seam were added. All four owned/borrowed × one/four-worker cases recorded zero owner-controller eventfd writes, with successful remote writes (2, 2, 38,052, 26,201). Those instrumented throughput values are excluded from every table here. See the [counter samples](../explorations/direct-ring-write-fastpath/counted/samples/aiko.json) and their archived instrumentation patch; this is not an instrumented build of the final executable.

## Artifacts

Every final timing sample has a uniquely named JSON file and stderr in its phase directory, alongside a complete aggregate `aiko.json`; no final sample metadata was reconstructed. `provenance.json` records all executable/source/runner hashes, toolchain, machine, affinity, and build settings. Prior experimental recovery is documented in its own directory and does not affect this fresh run.

Frozen final executable SHA-256: `4b4a6dd7f90ee5549738abff476d94b7c52bd098c1f133811fff13fdf57b63ad`. Original control SHA-256: `abc161cde925e54c6f3fb3d7e70d4d77f4f21e055877e293c27eb2a532e4834f`.
