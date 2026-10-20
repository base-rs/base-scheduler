# Combined readiness allocation experiment

**Decision: reject.** Three randomized repeats in each required workload show no convincing throughput improvement. Every before/after range overlaps. No source change is selected.

The driver and readiness receiver share one Arc allocation containing resources and Mutex-protected completion state. A private queued enum holds a compact Arc owning handle or existing boxed owned operation. This replaces readiness Box+oneshot with one allocation. Socket and capacity are removed under lock, released outside it, then terminal completion is published. Receiver Waker clone, replacement destruction and removal destruction happen outside the mutex. Failed-driver guards disconnect only after unknown in-flight Arcs are retained.

| Workers | Connections | Variant | Median req/s | Mean req/s | Range req/s | Median change | Median server user s | Median server system s |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 32 | before | 566,152 | 566,004 | 564,863–566,998 | +0.00% | 0.5141 | 2.2699 |
| 1 | 32 | combined | 562,307 | 564,029 | 561,817–567,963 | -0.68% | 0.5464 | 2.2387 |
| 4 | 128 | before | 2,061,173 | 2,061,711 | 2,060,186–2,063,774 | +0.00% | 2.8181 | 8.7337 |
| 4 | 128 | combined | 2,059,839 | 2,057,866 | 2,051,271–2,062,489 | -0.06% | 2.8008 | 8.7420 |

fixed client; frozen-seed randomized 3 repeats per variant/workload; 1 s warmup; 3 s measured; 64-byte validated payload; server pinned to CPU 0..workers-1, client to CPUs 4..7; exclusive benchmark lock. All binaries use the same optimized default release profile and frozen merged-main source `a98f172f8b06f23b34a354a4b6f6c294aa5859ab`. Throughput includes measurement only; CPU includes warmup and shutdown, so CPU seconds are not directly per measured request.

The candidate passed package unit tests, 180 conformance tests and one doctest; exact counts and output are in `tests.log` and `provenance.json`. Raw complete 12-sample data and runner/client/binary hashes are in `aiko.json`; exact source is in `source.patch`. Binary hashes remain in provenance after deleting build outputs.

This was a performance prototype, not merge-ready code. An unresolved exceptional-path concern remains: if releasing the socket/capacity panics (for example through a capacity waiter Waker), the shared readiness completion could stay Pending; the original local oneshot sender would disconnect on unwind. A borrowed disconnect-on-unwind guard should address this if the design is ever reconsidered. No guard or coverage expansion was added because throughput did not justify retaining the design.
