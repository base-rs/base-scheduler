# Owner-wake comparison with pure Tokio

The [subsequent readiness comparison](readiness-performance.md) measures later changes. The figures below remain the historical owner-wake measurements.

This comparison measures the native owner-wake improvements against pure Tokio and both Tokio adapter paths using the same frozen optimized server executable for all five backends. Production runtime source is `3af77be0f8b8a9303c8680a7f8aab49b1c3d304c`; server SHA-256 is `4b4a6dd7f90ee5549738abff476d94b7c52bd098c1f133811fff13fdf57b63ad`. The [raw report](results/socket-performance/owner-wake-vs-tokio/aiko.json) records its source/build provenance, fixed client, runner hashes, and all samples.

Aiko runs three randomized repetitions per backend and workload, with one second of warmup and three seconds of measurement. Server CPUs are 0 or 0–3; the four-worker client uses CPUs 4–7. Runs hold the exclusive benchmark lock. These 75 samples are separate from the earlier before/after native comparison and historical Tokio matrices.

## Throughput relative to pure Tokio

Percentages are ratios of median complete round trips per second; 100% means equal throughput to pure Tokio in this workload. The owned native path uses completion reads and attempts ready scalar writes synchronously. Its borrowed path uses nonblocking syscalls followed by ring readiness waits. Both Tokio adapter paths use readiness.

| Workers | Connections | Bytes | Pure Tokio requests/s | Tokio owned | Tokio borrowed | Native owned | Native borrowed |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 1 | 64 | 267,560 | 100.7% | 100.4% | 88.9% | 84.2% |
| 1 | 32 | 64 | 695,545 | 88.2% | 89.5% | 89.7% | 80.9% |
| 1 | 32 | 4096 | 615,263 | 90.7% | 89.6% | 90.4% | 82.3% |
| 4 | 128 | 64 | 2,104,109 | 95.1% | 94.9% | 93.2% | 93.5% |
| 4 | 128 | 4096 | 1,863,051 | 95.8% | 96.0% | 93.7% | 94.5% |

With four workers, the Tokio adapter reaches 94.9–96.0% of pure Tokio throughput and native reaches 93.2–94.5%. With one worker and 32 connections, the adapter reaches 88.2–90.7%, native owned reaches 89.7–90.4%, and native borrowed reaches 80.9–82.3%. The adapter therefore does not consistently meet the 5% target, and native remains behind pure Tokio in every measured workload. The single-connection adapter result is approximately equal to pure Tokio.

## Absolute throughput and variation

Minimum–maximum complete round trips per second across three repetitions, in millions. Full-precision medians and latency summaries are in the [summary](results/socket-performance/owner-wake-vs-tokio/summary.json).

| Workers/connections/bytes | Pure Tokio | Tokio owned | Tokio borrowed | Native owned | Native borrowed |
| --- | --- | --- | --- | --- | --- |
| 1/1/64 | 0.267–0.270 | 0.269–0.270 | 0.268–0.270 | 0.232–0.241 | 0.224–0.230 |
| 1/32/64 | 0.692–0.697 | 0.613–0.620 | 0.613–0.624 | 0.623–0.624 | 0.562–0.568 |
| 1/32/4096 | 0.613–0.620 | 0.557–0.559 | 0.549–0.555 | 0.555–0.557 | 0.501–0.510 |
| 4/128/64 | 2.100–2.114 | 1.996–2.006 | 1.990–1.997 | 1.961–1.968 | 1.954–1.972 |
| 4/128/4096 | 1.854–1.869 | 1.776–1.788 | 1.786–1.792 | 1.735–1.748 | 1.759–1.760 |

This is a loopback TCP echo workload with persistent connections and one outstanding request per connection. It does not establish general application performance or physical-network throughput. Percentages describe these concrete implementations and buffer paths, including scheduling and I/O, rather than isolating trait-dispatch cost. Three repetitions give a focused comparison rather than a guarantee about every run. Raw samples retain measured-window latency and lifecycle CPU use; no sample was removed or reconstructed.

The [owner-wake report](self-interrupts.md) describes the implementation and separate native before/after measurements. The [historical Tokio report](socket-performance.md) remains unchanged as a measurement of its earlier runtime.
