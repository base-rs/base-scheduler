# Working towards Tokio parity

The target is application throughput comparable to or better than ordinary raw Tokio, with correct cancellation, ownership, urgent-data handling, and cleanup. Faster individual operations or fewer explicit kernel entries do not establish that target. These fresh measurements expose a concurrent monitor regression in the ordered-writer experiment and retain a rejected receive-policy prototype.

## Fresh baseline

Forty optimized Aiko samples at `b70f46c` use five repetitions, one server worker, 64-byte messages, a fixed validating client, and matched server/client CPU affinity. Values are median complete round trips per second. All modes come from the same executable.

| Connections | Raw Tokio | Tokio adapter monitor | Native monitor | Native owned |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 273,912 | 273,451 | 273,928 | 220,749 |
| 32 | 710,038 | 647,473 | 572,810 | 622,399 |

Native monitors match the single-connection control but reach only 80.7% of raw Tokio with 32 connections. The owned path reaches 80.6% and 87.7%, respectively. The earlier approximately 14% owned-echo gap was a separate measurement window; it must not replace these fresh results. The application-level comparison includes different receive mechanisms and readiness policies. The goal remains ordinary Tokio parity even when a same-policy diagnostic explains part of the gap.

The separate instrumented run counts approximately two direct receive calls per Native monitor echo versus one for raw Tokio. Whole-server instructions per echo are approximately 17,063 for monitors, 16,382 for Native owned I/O, and 15,264 for Tokio. Monitors use almost no explicit ring entries in this busy workload, despite being slower. Explicit entry counts alone therefore fail to characterize the remaining cost. Instrumented throughput is excluded from the capacity tables.

## Ring policy regression

A startup-only diagnostic switch compares ordinary and cooperative task-work delivery in one executable. Both modes retain identical hot-path source, including the taskrun check. Thirty-six randomized samples use three repetitions per configuration.

| Connections / backend | Ordinary | Cooperative | Cooperative change |
| --- | ---: | ---: | ---: |
| 1 / Native monitor | 273,094 | 272,195 | −0.3% |
| 32 / Native monitor | 573,507 | 633,001 | +10.4% |
| 32 / Native owned | 621,982 | 626,370 | +0.7% |
| 32 / raw Tokio control | 714,149 | 711,994 | −0.3% |

All three cooperative concurrent-monitor samples exceed all three ordinary samples. Owned ranges overlap; their small difference does not establish a gain. The writer-focused ordinary-delivery improvement therefore came with a substantial regression in the recommended concurrent borrowed path.

Restore cooperative delivery only when an unused driver enters a thread-bound worker. Movable or previously driven drivers retain their existing ring, and optional setup failures retain the validated original. This restoration preserves the writer interface, cancellation allocation improvements, submission receipts, and CI fixes. It deliberately withdraws the global ordinary-delivery change from the ordered-writer implementation. The historical writer reports retain their measured results but no longer describe the final ring policy.

Twenty-four additional uninstrumented samples confirm the production-safe restoration at `bef1940`, including raw Tokio controls from both executables:

| Connections | Native before | Native restored | Raw Tokio before | Raw Tokio restored |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 273,672 | 273,941 | 271,470 | 275,135 |
| 32 | 572,941 | 631,291 | 706,283 | 710,991 |

The restored concurrent monitor improves 10.2% before adjusting for control drift and reaches 88.8% of its raw Tokio control. This removes the regression; it does not meet the parity target. Single-connection Native medians are unchanged within this small sample's variation.

## Rejected one-shot rearming

A second same-executable experiment uses the existing stable monitor owner with a fresh one-shot level poll after a positive short read. This avoids the old one-shot path's request allocation and reply channel. A fresh kernel level check also avoids the queued urgent-data suffix problem caused by clearing an edge-triggered cached event after a short read.

The prototype passes targeted urgent-data, peek, empty-read, datagram, cancellation, dropped-waiter, migration, and independently driven conformance tests. It changes only scalar, nonempty, non-peek reads on actual readable stream monitors. Forty-eight randomized samples independently vary rearming and ring policy, with raw Tokio controls in every combination.

| 32 connections | Persistent monitor | One-shot rearming | Change |
| --- | ---: | ---: | ---: |
| Ordinary task work | 571,754 | 616,076 | +7.8% |
| Cooperative task work | 631,712 | 607,761 | −3.8% |

Rearming helps the slower ordinary-delivery path but loses to persistent monitoring with cooperative delivery. Single-connection results vary and do not establish a gain. Keep this prototype experimental; it is not a default-policy recommendation. Startup environment switches exist only in diagnostic patches and do not ship in the production restoration.

## Evidence and reproduction

All runs use Aiko's Ryzen 9950X3D, Linux 7.2.8, Rust 1.99.0, locked dependencies, and `CARGO_PROFILE_RELEASE_DEBUG=1 cargo build --release`. No LTO or target-CPU override is added. Server CPU 0 and client CPUs 4–7 are separate physical cores in one last-level cache. Each capacity sample uses one warmup second and three measured seconds. Every response is validated, buffers are reused, and all builds, tests, profiles, and timings serialize on `/tmp/core-scheduler-perf.lock`.

[Raw baseline samples](results/parity/baseline.json), [same-executable ring controls](results/parity/policy.json), [production restoration confirmation](results/parity/restored.json), and [rearming samples](results/parity/rearm.json) retain all repetitions and binary hashes. [The diagnostic policy patch](results/parity/policy-control.patch) and [combined rearming patch](results/parity/stream-rearm.patch) preserve their source. Rearming source `ddf6c3c` differs from its measured production source `982ff6d` only by private test-call updates.

[The policy runner](results/parity/policy.py) and [frozen-executable comparison runner](results/parity/confirm.py) reuse the canonical echo sampler. For the policy runner, set `--source-root` to the source checkout used to build the chosen executable; `--compare-rearm` adds the independent rearming dimension. The frozen-executable runner records each source revision supplied through `--variants`. Preserve separate Cargo target directories for different source variants. The frozen client has SHA-256 `c72f7aea7151fb83020b096cfdaccc46a70b0eeff6bbb0a0e4ea4157d1aeb071`.

[The profile runner](results/parity/profile.py), [counter files](results/parity/profiles), and profile sample metadata preserve the baseline diagnostics. Large raw perf traces remain on Aiko under `/home/samuel/.cache/core-scheduler-tokio-parity/profiles`. A separate attempted cooperative profile lost its startup environment through `sudo`; its remote metadata marks it as a repeated normal-mode run excluded from cooperative comparisons. The unprivileged capacity runner passes the policy directly to the server and does not have that issue.

The exact production restoration passes the full Linux workspace coverage gate, 34,574/34,574 measured regions with twelve invariant exclusions, and strict workspace Clippy on Linux and macOS. Independent review checked movable and already-driven rings, queued owners, buffer/file registration, and writer submission receipts. Measurements remain short closed-loop loopback tests; wider workloads and longer repetitions are necessary before a general parity claim.
