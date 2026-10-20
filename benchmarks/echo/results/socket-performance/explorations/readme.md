# Socket performance explorations

These are separate experiments, with their own controls and source patches. They are not a pooled benchmark. The current [owner-wake report](../../../self-interrupts.md) and [final confirmation](../owner-wake-final) identify the integrated runtime.

This archive retains text evidence, including source patches, hashes, runners, raw JSON, and logs. Frozen executable and build outputs remain in Aiko's caches. Paths inside original provenance files name those remote locations; directory names below are shorter archival names.

| Directory | Experiment |
| --- | --- |
| `self-wake-suppression` | Initial worker-level owner-wake suppression, rejected for throughput regression. |
| `worker-local-permit` | Retained local permits, still regressing. |
| `selector-local-notification` | Exact-controller local flag with the original wait path; writes removed but throughput regressed. |
| `selector-level-waits` | Level-triggered wait experiment. |
| `completion-after-submission` | Completion collection after submission. |
| `direct-ring-busy-loop` | Invalid performance candidate: notification rearming caused an idle busy loop. Read `INVALID-PERFORMANCE.md` before interpreting any rates. |
| `direct-ring-corrected` | Drain-before-rearm fix; idle behavior corrected, owned throughput still slower. |
| `direct-ring-owned-retry` | Retry readiness on the original ring. |
| `direct-ring-combined-wait` | Combining idle submission with waiting. |
| `direct-ring-scalar-fastpath` | Read-and-write synchronous attempts, rejected. |
| `direct-ring-write-fastpath` | Write-only candidate, including separate counters and explicitly labelled recovered preliminary matrix. |

The existing `idle-regression` and `lazy-cancellation` directories predate this follow-up and remain unchanged.
