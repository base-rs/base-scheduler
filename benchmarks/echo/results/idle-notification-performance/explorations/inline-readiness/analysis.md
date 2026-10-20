# Inline readiness allocation experiment

**Decision: reject.** Three randomized repeats in each required workload show no convincing throughput improvement. Every before/after range overlaps. No source change is selected.

ReadinessRequest moves inline inside a private QueuedRequest enum, removing its Box. Owned scalar and message requests remain boxed and stable. Command, pending and in-flight storage use the enum. Release-before-wake, cancellation routing and failed-driver retention remain unchanged.

| Workers | Connections | Variant | Median req/s | Mean req/s | Range req/s | Median change | Median server user s | Median server system s |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 32 | before | 564,825 | 565,006 | 562,848–567,345 | +0.00% | 0.5354 | 2.2431 |
| 1 | 32 | inline | 564,168 | 563,677 | 562,041–564,823 | -0.12% | 0.5431 | 2.2393 |
| 4 | 128 | before | 2,059,491 | 2,060,490 | 2,050,413–2,071,567 | +0.00% | 2.7640 | 8.8007 |
| 4 | 128 | inline | 2,061,680 | 2,061,056 | 2,053,079–2,068,410 | +0.11% | 2.7336 | 8.7814 |

fixed client; frozen-seed randomized 3 repeats per variant/workload; 1 s warmup; 3 s measured; 64-byte validated payload; server pinned to CPU 0..workers-1, client to CPUs 4..7; exclusive benchmark lock. All binaries use the same optimized default release profile and frozen merged-main source `a98f172f8b06f23b34a354a4b6f6c294aa5859ab`. Throughput includes measurement only; CPU includes warmup and shutdown, so CPU seconds are not directly per measured request.

The candidate passed package unit tests, 180 conformance tests and one doctest; exact counts and output are in `tests.log` and `provenance.json`. Raw complete 12-sample data and runner/client/binary hashes are in `aiko.json`; exact source is in `source.patch`. Binary hashes remain in provenance after deleting build outputs.

No additional production change is selected; no coverage expansion was performed for this rejected prototype.
