# Borrowed io_uring controller routing

The selector already owns each worker controller in an immutable vector. TLS now returns the worker index for the explicitly matching DriverGroup. Borrowed monitor operations retain a reference to that selector-owned Arc rather than incrementing/decrementing it for each operation. The active ring binding continues owning its controller during cancellation, retirement, abandonment and migration. No new cache, allocation or unsafe code is added. Owned completion operations still clone their controller where they require independent retention.

The final cleanup uses the same route_index helper for worker-local buffer-pool selection, eliminating its clone-to-read-worker step and the duplicate TLS lookup helper. The focused regression test checks external round-robin selection, unchanged controller strong counts, nested selector identity with different worker indices, and a retained reference crossing threads.

## Screening

Aiko, release with debug symbols and default workspace optimizations. Fixed client; CPUs 0–3 for servers and 4–7 for clients; 1-second warmup, 3 measured seconds. Five randomized repetitions per variant and case, 75 total samples. Entire run holds /tmp/core-scheduler-perf.lock. Before and after use the same exclusive-monitor baseline `1840258` + monitor blob `7283980a9fc48bdbdb786117d7505e211c689851`. The post-review cleanup guard is tested separately, so this screen isolates the borrowed routing change. The later buffer-pool helper cleanup is outside the measured borrowed echo path.

| Workers | Connections | Bytes | Exclusive req/s | Borrowed route req/s | Change | Raw Tokio req/s |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| 1 | 1 | 64 | 274,422 | 273,097 | -0.48% | 274,428 |
| 4 | 1 | 64 | 255,488 | 250,833 | -1.82% | 276,131 |
| 1 | 32 | 64 | 633,257 | 636,041 | +0.44% | 715,210 |
| 4 | 32 | 64 | 1,760,543 | 1,752,199 | -0.47% | 1,748,192 |
| 1 | 32 | 4096 | 576,532 | 577,999 | +0.25% | 632,504 |

No meaningful throughput improvement or clear regression is established. The 4-worker/1-connection baseline spans 249,555–269,564 req/s, wider than its 1.82% median difference. Prefer this change for eliminating unnecessary ownership with a simple reference-based model, without claiming a substantial performance gain. The final combined matrix is reported in [the investigation](../../../tokio-parity.md).

The earlier profile's 10.69% route attribution was userspace-only; that run consumed 0.566s userspace versus 3.766s system CPU. It never established a 10% total CPU or throughput opportunity.

Aiko had unrelated game/Wine processes active, last observed on CPU 11 with broad 0–31 affinity. Their affinity and governor were not changed. Randomized matched controls remained broadly stable; small differences should not be over-interpreted.

## Validation

The borrowed route plus the generation-based Wait cleanup guard passes the complete Linux core-scheduler-iouring suite: 216 unit tests, 3 file-resource tests, 273 conformance tests, 11 resource-options tests, 2 shared-pool tests and 1 doctest (506 total). This includes active-monitor migration, cancellation, retirement and the new abandoned-task ownership regression. The final shared route_index buffer-pool cleanup also passes the same complete suite.

The raw samples and executable hashes are in `route-screen.json`; `route-screen.patch` records the screened source. The shared comparison runner is `../compare.py`.
