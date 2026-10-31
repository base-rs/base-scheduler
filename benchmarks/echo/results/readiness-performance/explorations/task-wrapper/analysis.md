# Task wrapper diagnostic

The scheduler task wrapper does not explain the echo gap. Bypassing `TaskService`, `TaskFuture`, heap pinning, panic guards, task registration, and owning task handle semantics improves median throughput by only 0.29% for owned adapter I/O and 0.64% for borrowed I/O. The adapter remains about 11% below pure Tokio.

This diagnostic duplicates the scheduler server function and replaces only `scheduler.spawn` with `tokio::spawn`; adapter I/O and all workload settings are retained. Every backend runs from one optimized executable. No runtime source code changed. Three randomized repetitions, 1 worker on CPU 0, 32 connections, 64-byte payload, 1-second warmup, 3-second measured interval, fixed four-worker client on CPUs 4–7. Exclusive `/tmp/core-scheduler-perf.lock` throughout. All samples validate payload and complete with empty stderr.

| Backend | Median requests/s | Pure Tokio | Range |
| --- | ---: | ---: | ---: |
| tokio | 699,326 | 100.00% | 695,682–706,266 |
| tokio-adapter | 621,289 | 88.84% | 620,277–622,731 |
| tokio-adapter-rawtask | 623,076 | 89.10% | 620,248–624,816 |
| tokio-adapter-poll | 620,152 | 88.68% | 614,866–620,991 |
| tokio-adapter-rawtask-poll | 624,114 | 89.25% | 621,700–624,466 |

The TaskFuture profile symbol includes the inlined echo implementation, so its self percentage does not isolate wrapper overhead. The wrapper adds one heap allocation at task creation and one pointer indirection during polling; neither is per I/O request. The panic-catching wrapper remains worthwhile for cleanup and panic semantics. Inline pinned storage would introduce unsafe bookkeeping without evidence that it addresses the remaining performance gap.

This is a short diagnostic, not a claimed universal bound or a production change. Preserve the existing safe task wrapper. The raw-task benchmark patch is evidence only and should not ship.

`provenance.json` records source, executable/patch/runner/client hashes, compiler, build command and CPU affinity. `rawtask-screen/aiko.json` contains every measured sample. `rawtask.patch` records diagnostic source.
