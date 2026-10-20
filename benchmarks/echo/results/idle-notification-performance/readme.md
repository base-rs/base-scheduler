# Idle notification performance evidence

The [report](../../idle-notification-performance.md) describes the selected optimization and its limits. All experiments start from merged PR #14, `a98f172f8b06f23b34a354a4b6f6c294aa5859ab`, unless an individual provenance file says otherwise. Rejected patches are evidence only; they are not applied to the workspace.

The [final comparison](final/aiko.json) measures only the selected change. Its [provenance](final/provenance.json) identifies the exact source, optimized executables, fixed client, compiler, and host. Run `python3 benchmarks/echo/results/idle-notification-performance/final/summarize.py` from the checkout to validate the sample matrix and regenerate its summary tables. Individual JSON and empty stderr files are preserved for every final sample. Binaries, targets, coverage data, and disassembly are excluded from this archive.

## Explorations

Each `aiko.json` preserves the complete screen, including samples, executable hashes, runner/helper hashes, workload, timing, and affinity. Most screens use three repetitions; they can reject an unhelpful candidate but do not establish small differences reliably. Throughput comparisons use matched workload medians. CPU fields measure the server's full lifecycle, including warmup.

| Directory | Candidate | Outcome |
| --- | --- | --- |
| [early-idle-poll](explorations/early-idle-poll/aiko.json) | Remove the early nonblocking turn; additionally combine submission and wait. | Rejected: borrowed single-connection throughput fell about 13% and 10%, respectively, and four-worker throughput fell about 3%. |
| [local-wake](explorations/local-wake/aiko.json) | Suppress local ring notifications while polling tasks. | Rejected: no useful throughput gain; suppressing peer wakeups can delay independent task fanout. |
| [completion-after-wait](explorations/completion-after-wait/aiko.json) | Collect CQ immediately after the blocking wait. | Rejected: single-connection throughput fell about 12%; no concurrent benefit. Publishing wakes while the executor still marks the worker sleeping also requests redundant interrupts. |
| [ring-flags](explorations/ring-flags/aiko.json) | Use deferred task work and single-issuer rings. | Rejected: roughly 2% four-worker gain accompanied a roughly 3% serial reduction. |
| [owner-wake](explorations/owner-wake/aiko.json) | Exclude the owner from selected scheduler wakes, separately and with immediate CQ collection. | Rejected: no concurrent gain. The general selector contract can publish callbacks then block, relying on the task waker to interrupt it; owner-wake suppression requires a stronger contract. |
| [eventfd-pilot](explorations/eventfd-pilot/aiko.json) | Remove the speculative idle eventfd read. | Selected for the larger final matrix: the 36-sample pilot showed about 5–7% serial improvement with concurrent throughput approximately unchanged. |
| [hash](explorations/hash/analysis.md) | Use a specialized hash for private request IDs. | Rejected: all matched median gains below 1%, with additional hash complexity and poor low-bit stride distribution. |
| [local-queue](explorations/local-queue/analysis.md) | Capture the thread-local request route once per reactor turn. | Rejected: matched changes between −0.41% and +0.12%. |
| [inline-readiness](explorations/inline-readiness/analysis.md) | Store readiness requests inline while retaining the completion channel. | Rejected: borrowed throughput changes of −0.12% and +0.11% at one and four workers. |
| [combined-readiness](explorations/combined-readiness/analysis.md) | Combine readiness storage and completion into one shared allocation. | Rejected: borrowed throughput changes of −0.68% and −0.06%, with substantially more code. Its unresolved unwind-disconnection concern is documented for any future revisit. |
| [Tokio](explorations/tokio/analysis.md) | Simplify waiter/future state and inline cancellation or syscall/admission wrappers. | Rejected: no useful owned-path gain; call elimination did not translate to a robust throughput gain. |
| [thin LTO](explorations/tokio/thin-lto-analysis.md) | Build the unchanged baseline with thin LTO and compare all five backends using the same profile. | Diagnostic only: +0.09% to +1.21% across medians; adapter ratios remain about 92–93% of raw Tokio in this workload. The default release profile is unchanged. |

For the six root screens above, each patch is relative to the common base, rather than cumulative with another candidate. The final runner and helper are also the frozen versions used by these screens; their hashes are checked against the raw reports. The hash and local-queue archives retain their own runner, provenance, and checksums. Their `aiko.json` files were flattened from the original remote output directories when archived.
