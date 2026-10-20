# Alternating native ready queues

The experiment improves throughput and tail latency in the tested concurrent echo workloads, especially with four workers, but repeats a small owned-I/O serial throughput regression. Keep it experimental rather than treating it as a regression-free replacement for the existing loop.

## Worker policy

Each worker owns two reusable, stealable FIFO queues. At a batch boundary it admits external work, takes the first runnable, and swaps the remaining ready queue into the current batch. All local wakes during execution go into the next queue. It drains the finite current batch, publishes the available I/O completion batch through a nonblocking driver turn, and repeats. When idle, it publishes its sleeping state, rechecks runnable queues, and enters the interruptible driver wait. It does not make an additional nonblocking driver probe immediately before that wait.

Both physical queues retain stable stealers while their local roles swap. Another worker can steal from either generation; these are independent worker batches, not globally synchronized rounds. Queue selection, external admission, and fan-out checks move to batch boundaries. Task ownership, runnable accounting, panic containment, cancellation, kernel-buffer retention, and sleeping/wakeup coordination remain in place. Queue storage is reused; forming a batch does not allocate or copy all its runnables.

The experiment removes the 61-task event interval. A task waking itself cannot extend this worker's current batch, but there is no maximum batch size: a very large batch can delay I/O checks. A single future poll that never returns still occupies its worker. This experiment adds no cooperative operation budget.

## Final source comparison

Baseline source is `ddc994d2f1f3f10df53e46241a730d38a7c37531` (the scheduler simplicity PR), not an older main-branch baseline. Final measured candidate source is `d1306dfc7ccde4502d4e966cc4e070bc32d2e101`, tree `16befc49202178cbe2bd5473968fc0c4ba2b7aba`. Subsequent commits add documentation and evidence only. The remote staged source tree was checked against the local committed tree before measurement. The [source patch](results/double-buffered-ready/candidate.patch) contains the five changed Native source/test files.

The [final 90 samples](results/double-buffered-ready/final/aiko.json) contain three interleaved repetitions per build/backend/workload. Throughput cells show median requests/s and the observed minimum–maximum. The latency column shows the median of each run's p99 RTT, not a pooled p99.

| Workers/connections/bytes | Backend | Before requests/s [range] | After requests/s [range] | Change | Median per-run p99 µs, before → after |
| --- | --- | ---: | ---: | ---: | ---: |
| 1/1/64 | tokio | 271,967 [271,928–274,867] | 276,120 [271,962–276,463] | +1.53% | 4.311 → 4.291 |
| 1/1/64 | iouring | 240,202 [232,491–241,943] | 233,270 [229,382–233,272] | -2.89% | 5.791 → 6.011 |
| 1/1/64 | iouring-poll | 232,476 [231,525–234,402] | 233,396 [230,355–234,888] | +0.40% | 5.791 → 5.843 |
| 1/32/64 | tokio | 691,191 [690,829–699,132] | 690,927 [689,195–698,268] | -0.04% | 54.111 → 54.143 |
| 1/32/64 | iouring | 628,936 [627,412–629,618] | 634,014 [632,981–635,870] | +0.81% | 57.919 → 54.399 |
| 1/32/64 | iouring-poll | 562,374 [561,430–568,131] | 564,353 [563,529–565,993] | +0.35% | 62.847 → 60.927 |
| 1/32/4096 | tokio | 608,732 [608,713–619,123] | 617,093 [614,462–623,774] | +1.37% | 61.087 → 60.319 |
| 1/32/4096 | iouring | 558,988 [558,941–561,598] | 564,459 [562,128–566,667] | +0.98% | 65.503 → 61.311 |
| 1/32/4096 | iouring-poll | 512,510 [507,218–515,815] | 506,485 [503,353–510,200] | -1.18% | 68.607 → 67.903 |
| 4/128/64 | tokio | 2,111,263 [2,108,611–2,114,702] | 2,096,574 [2,085,363–2,113,815] | -0.70% | 102.335 → 103.423 |
| 4/128/64 | iouring | 2,052,581 [2,040,285–2,063,165] | 2,080,845 [2,065,431–2,086,633] | +1.38% | 113.727 → 108.799 |
| 4/128/64 | iouring-poll | 2,054,903 [2,044,755–2,056,900] | 2,116,994 [2,094,270–2,129,868] | +3.02% | 117.759 → 105.023 |
| 4/128/4096 | tokio | 1,864,548 [1,862,295–1,866,714] | 1,846,323 [1,836,317–1,853,798] | -0.98% | 115.455 → 118.655 |
| 4/128/4096 | iouring | 1,811,090 [1,809,887–1,813,132] | 1,842,946 [1,831,007–1,848,798] | +1.76% | 126.911 → 120.895 |
| 4/128/4096 | iouring-poll | 1,823,723 [1,819,331–1,823,948] | 1,866,412 [1,862,329–1,871,098] | +2.34% | 131.007 → 118.975 |

With four workers and 128 connections, owned throughput rises by 1.38–1.76% and borrowed `_poll` throughput by 2.34–3.02%. Concurrent Native median per-run p99 improves in every tested final-matrix cell. Borrowed throughput is about 1% above the same candidate executable's raw Tokio at four workers, while owned throughput is within 1%. These small margins do not establish a general advantage over Tokio: Native remains materially slower with one worker, and this is a short loopback workload.

One-worker concurrent throughput changes are small and mixed. Raw Tokio, whose implementation was not changed, moves from −0.98% to +1.53% across the final matrix. In particular, the owned 4 KiB one-worker gain is smaller than that control's change. Do not treat all sub-percent differences as established speedups.

## Selected serial confirmation

Owned serial throughput declines by 4.21% in the initial prototype matrix and 2.89% in the final-source matrix. Because the serial ranges are wide, a separate [20-sample confirmation](results/double-buffered-ready/serial-confirmation/aiko.json) repeats the final source and baseline five times each alongside raw Tokio. This workload was selected after examining the earlier measurements and is kept separate.

| Workers/connections/bytes | Backend | Before requests/s [range] | After requests/s [range] | Change | Median per-run p99 µs, before → after |
| --- | --- | ---: | ---: | ---: | ---: |
| 1/1/64 | tokio | 271,245 [268,799–274,415] | 269,906 [268,534–270,656] | -0.49% | 4.351 → 4.371 |
| 1/1/64 | iouring | 236,126 [231,542–241,883] | 230,357 [226,842–234,925] | -2.44% | 6.311 → 9.543 |

The repeated owned serial decline is a concern despite overlapping throughput ranges. Tail latency is also variable and worsens in this confirmation; the evidence does not support claiming no serial regression. This matrix alone does not identify which executor cost dominates. A [subsequent targeted diagnosis](results/double-buffered-ready/diagnosis/readme.md) finds redundant worker self-notifications during idle completion publication: approximately four driver turns and seven queue searches per echo, versus three and four in the original loop. Suppressing that self-notification recovers the loss in a separate selected serial experiment. Skipping singleton queue swaps or reducing external-queue checks did not recover it in their screen. These diagnostic variants have not been applied as a production fix or tested across this concurrent matrix.

## Initial prototype and idle checks

The [initial 90 samples](results/double-buffered-ready/initial/aiko.json) measure candidate `bc4cde1b907f269cc31e7493fa2e49ee919a56e0`, tree `c8bbddbc2d822e11ce426e22b7e31b00c760ee17`. That version retained a redundant nonblocking driver check before the idle wait. Before the final comparison, the implementation removed that check and released its mutable thread-local queue borrow before invoking worker-wakeup callbacks. The two matrices are not pooled.

The initial concurrent results broadly agree with the final direction: four-worker owned gains of 0.84–1.81% and borrowed gains of 2.94–3.15%. Serial owned throughput was lower in both matrices.

The [idle check](results/double-buffered-ready/idle.json) starts twelve servers: baseline/candidate, one/four workers, and raw Tokio/owned Native/borrowed Native. Each warms for half a second and is observed without clients for two seconds. Every sample records zero process CPU ticks during that interval, so no idle spin was observed at that measurement resolution.

## Validation

The [validation record](results/double-buffered-ready/validation.json) covers the final measured source:

- macOS workspace tests, all targets/features: 15,853/15,853 measured source regions covered.
- Linux Native tests, all targets/features in an isolated coverage directory: 1,329/1,329 regions covered.
- Linux io_uring unit, integration, conformance, and documentation tests passed.
- Workspace Clippy with all targets/features and warnings denied passed on macOS; formatting passed.
- New regression tests verify that a self-wake waits for the next driver turn on a single worker and that both ready generations remain stealable. Existing tests cover completion-batch publication, busy-task I/O progress, incoming wakes, driver failure/retirement, cancellation, and shutdown.

Windows was not run for this experiment. No platform-specific source or public capability signature changed. A first Linux coverage invocation reused artifacts from other checkouts and included unrelated binaries; it was discarded and rerun with an isolated coverage directory. The successful isolated result above is the reported check.

## Reproduction and limits

Both servers use `cargo build --release --locked -p core-scheduler-echo-benchmark`, Cargo's default optimized release profile (`opt-level=3`, no LTO or target-CPU override). The [environment snapshot](results/double-buffered-ready/environment.json) records no Cargo configuration files or profile/flag overrides and powersave governors on the selected cores. The result JSON records compiler/kernel versions, CPU topology, exact source trees, executable hashes, and the fixed client's identity. The client comes from `5736301fded46c4e5f11d5387950ae2ee0807b94`, SHA-256 `db6c914ab75011477ec6232e681046b1e75532cbaa1c81663bac984591675cf3`.

The [runner](results/double-buffered-ready/run.py) and [sample helper](results/double-buffered-ready/sample-runner.py) preserve the timing procedure. Build each source into a separate frozen executable. Run the runner with `--output DIR --build BUILD_JSON --client CLIENT --repeats 3`, before/after `--case LABEL,BACKEND,BINARY` entries for `tokio`, `iouring`, and `iouring-poll`, and these five repeatable workloads: `--workload 1,1,64`, `--workload 1,32,64`, `--workload 1,32,4096`, `--workload 4,128,64`, and `--workload 4,128,4096`. `BUILD_JSON` supplies source/build provenance. Use only the serial workload, the Tokio and owned cases, and five repetitions to reproduce the selected confirmation. The [summary script](results/double-buffered-ready/summarize.py) generates the tables. The [idle probe](results/double-buffered-ready/idle.py) preserves its separate check.

Samples use one second of warmup and three seconds of measurement, randomized interleaving, server CPUs 0 or 0–3, and client CPUs 4–7. An exclusive `/tmp/core-scheduler-perf.lock` prevents cooperating builds, tests, profilers, and other benchmark phases from overlapping timing; it does not exclude unrelated host activity. The workload is persistent IPv4 loopback TCP echo, `TCP_NODELAY`, reusable buffers, validated payloads, and one outstanding request per connection. RTT includes client scheduling. CPU totals in the timing samples cover process lifetimes rather than just the measurement window.

These short repetitions provide observed ranges, not a statistical equivalence test. Very large ready batches, fairness latency under sustained task spawning, the Native readiness backend's performance, physical networks, and general application workloads remain unmeasured.
