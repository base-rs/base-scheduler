# Scheduler simplicity pass

This pass removes unused task-ID bookkeeping, duplicate work-search types, temporary parking callback borrowing, repeated command-state updates, and duplicate socket-mode setup. One successful eventfd read clears its accumulated counter; notification processing no longer attempts a second speculative read. Task ownership, runnable retirement, parking fences, cancellation, completion batching, and descriptor lifetimes remain intact.

## Comparison

The initial matrix shows Native throughput changes from −1.25% to +1.65% and adapter changes from −0.82% to +0.80%. The unchanged pure-Tokio control moves from −1.76% to −0.31%. Two small adapter declines with non-overlapping ranges and the uncertain Native serial p99 increase were selected for further measurement. The separate confirmations below did not reproduce those declines. No repeatable throughput or tail-latency regression is evident in these tested workloads; the measurements do not establish a general speedup.

The existing performance gap remains: at one worker and 32 connections, the adapters are 6.26–7.63% below raw Tokio in the initial matrix; at four workers they are 2.54–4.12% below it.

The [complete 150-sample results](results/simplicity-performance/aiko.json) contain three interleaved repetitions per backend/workload/build. Throughput cells show median requests/s followed by the observed minimum–maximum. Positive change means higher throughput. The last column shows the median of per-run p99 RTTs in microseconds, not pooled p99.

| Workers/connections/bytes | Backend | Before requests/s [range] | After requests/s [range] | Change | p99 RTT µs, before → after |
| --- | --- | ---: | ---: | ---: | ---: |
| 1/1/64 | tokio | 271,272 [266,862–272,171] | 270,436 [269,047–272,122] | -0.31% | 4.33 → 4.35 |
| 1/1/64 | tokio-adapter | 268,691 [266,800–272,473] | 269,004 [264,626–270,324] | +0.12% | 4.45 → 4.36 |
| 1/1/64 | tokio-adapter-poll | 267,727 [265,857–272,920] | 269,878 [268,247–270,236] | +0.80% | 4.66 → 4.38 |
| 1/1/64 | iouring | 242,642 [237,219–255,574] | 239,604 [239,138–241,917] | -1.25% | 5.79 → 8.45 |
| 1/1/64 | iouring-poll | 237,653 [234,036–242,033] | 236,157 [233,190–237,899] | -0.63% | 6.33 → 5.87 |
| 1/32/64 | tokio | 704,066 [696,675–710,306] | 695,715 [690,701–697,621] | -1.19% | 53.31 → 54.11 |
| 1/32/64 | tokio-adapter | 639,324 [633,948–639,402] | 642,610 [640,718–646,074] | +0.51% | 57.98 → 57.63 |
| 1/32/64 | tokio-adapter-poll | 641,937 [632,720–646,995] | 643,425 [637,501–646,700] | +0.23% | 57.63 → 57.63 |
| 1/32/64 | iouring | 623,173 [619,875–631,242] | 633,430 [631,324–637,358] | +1.65% | 58.37 → 57.44 |
| 1/32/64 | iouring-poll | 566,439 [559,900–566,488] | 564,458 [559,302–565,827] | -0.35% | 62.49 → 62.62 |
| 1/32/4096 | tokio | 621,008 [617,501–621,276] | 610,065 [609,662–617,534] | -1.76% | 60.03 → 61.05 |
| 1/32/4096 | tokio-adapter | 571,529 [570,962–573,575] | 567,625 [562,916–568,904] | -0.68% | 64.51 → 64.93 |
| 1/32/4096 | tokio-adapter-poll | 571,002 [568,986–574,743] | 571,872 [566,578–576,630] | +0.15% | 64.54 → 64.42 |
| 1/32/4096 | iouring | 557,554 [556,145–561,706] | 559,378 [558,595–560,241] | +0.33% | 65.79 → 65.50 |
| 1/32/4096 | iouring-poll | 510,313 [506,574–514,441] | 511,225 [502,814–514,008] | +0.18% | 69.06 → 68.80 |
| 4/128/64 | tokio | 2,116,236 [2,097,815–2,123,153] | 2,105,713 [2,102,742–2,105,750] | -0.50% | 100.67 → 102.72 |
| 4/128/64 | tokio-adapter | 2,035,556 [2,027,276–2,048,827] | 2,018,888 [2,018,085–2,023,898] | -0.82% | 103.30 → 106.56 |
| 4/128/64 | tokio-adapter-poll | 2,024,988 [2,024,892–2,031,030] | 2,039,748 [2,035,315–2,042,014] | +0.73% | 108.67 → 103.68 |
| 4/128/64 | iouring | 2,063,049 [2,053,804–2,063,327] | 2,042,970 [2,042,848–2,060,716] | -0.97% | 112.25 → 114.24 |
| 4/128/64 | iouring-poll | 2,054,807 [2,049,368–2,058,534] | 2,052,664 [2,047,931–2,060,459] | -0.10% | 118.40 → 118.21 |
| 4/128/4096 | tokio | 1,857,122 [1,855,927–1,858,332] | 1,847,694 [1,832,992–1,859,340] | -0.51% | 118.33 → 116.29 |
| 4/128/4096 | tokio-adapter | 1,809,773 [1,785,103–1,810,199] | 1,796,696 [1,776,981–1,809,899] | -0.72% | 115.65 → 119.61 |
| 4/128/4096 | tokio-adapter-poll | 1,809,587 [1,800,987–1,814,298] | 1,800,785 [1,799,388–1,810,958] | -0.49% | 115.07 → 116.22 |
| 4/128/4096 | iouring | 1,805,191 [1,803,108–1,814,940] | 1,811,764 [1,799,590–1,818,600] | +0.36% | 127.42 → 126.72 |
| 4/128/4096 | iouring-poll | 1,825,274 [1,814,289–1,832,420] | 1,824,251 [1,823,297–1,827,520] | -0.06% | 127.94 → 130.37 |

## Selected confirmations

These followups use the same frozen executables, client, procedure, and lock, with five repetitions per cell. They remain separate from the initial matrix because their workloads were selected after inspecting it.

The [40-sample adapter confirmation](results/simplicity-performance/confirmation.json) repeats the two initially declining adapter cells alongside raw Tokio. Both before/after throughput ranges now overlap: adapter median changes are −0.15% and +0.66%, with control changes of −0.17% and +0.30%.

| Workers/connections/bytes | Backend | Before requests/s [range] | After requests/s [range] | Change | p99 RTT µs [range], before → after |
| --- | --- | ---: | ---: | ---: | ---: |
| 1/32/4096 | tokio | 620,718 [617,691–623,056] | 619,636 [617,565–622,679] | -0.17% | 60.031 [59.807–60.287] → 60.191 [59.839–60.351] |
| 1/32/4096 | tokio-adapter | 572,189 [570,836–574,718] | 571,326 [567,611–574,598] | -0.15% | 64.415 [64.127–64.447] → 64.479 [63.967–64.767] |
| 4/128/64 | tokio | 2,112,296 [2,085,209–2,131,821] | 2,118,567 [2,100,257–2,131,830] | +0.30% | 101.567 [99.903–104.127] → 102.079 [99.327–105.023] |
| 4/128/64 | tokio-adapter | 2,023,901 [2,014,925–2,046,615] | 2,037,343 [2,019,633–2,044,833] | +0.66% | 107.327 [102.527–110.079] → 104.319 [102.015–108.991] |

The initial owned Native serial p99 ranges were 5.603–9.375 µs before and 6.091–9.287 µs after, despite different medians. The [20-sample serial confirmation](results/simplicity-performance/serial-confirmation.json) does not reproduce that median increase: Native throughput changes by +2.43% and median p99 changes from 6.091 to 5.791 µs, with overlapping throughput and p99 ranges. This remains too variable to claim a reliable latency improvement.

| Workers/connections/bytes | Backend | Before requests/s [range] | After requests/s [range] | Change | p99 RTT µs [range], before → after |
| --- | --- | ---: | ---: | ---: | ---: |
| 1/1/64 | tokio | 270,052 [269,275–273,007] | 270,769 [269,833–272,395] | +0.27% | 4.351 [4.323–4.371] → 4.331 [4.311–4.363] |
| 1/1/64 | iouring | 238,946 [234,181–246,589] | 244,743 [236,814–247,889] | +2.43% | 6.091 [5.731–9.127] → 5.791 [5.731–7.431] |

## Reproduction and limits

Before source is `80188cf0825ca08874be4e97f28a13e21f216657`; measured candidate source is `7ebd1245456900d1d28f89fcb4198c800048cd38`, tree `72114e7c6816b6f83456f6e721fc4e6b004685f3`. Later changes in this PR add benchmark evidence only. The baseline was built from a clean checkout, and the candidate tree was recorded before its build. The aggregate records compiler, source identifiers, binary hashes, fixed-client identity, CPU topology, and exact runner/helper hashes.

Both servers use `cargo build --release --locked -p core-scheduler-echo-benchmark`, the default optimized release profile (`opt-level=3`), without LTO or target-CPU overrides. The [environment snapshot](results/simplicity-performance/environment.json) found no Cargo configuration files or profile/flag overrides; the pinned cores used the powersave governor. The fixed client comes from `5736301fded46c4e5f11d5387950ae2ee0807b94` and has SHA-256 `db6c914ab75011477ec6232e681046b1e75532cbaa1c81663bac984591675cf3`.

The [runner](results/simplicity-performance/run.py) and [sample helper](results/simplicity-performance/sample-runner.py) preserve the exact procedure. Each sample uses one second of warmup and three seconds of measurement, with server CPUs 0 or 0–3 and client CPUs 4–7. An exclusive performance lock prevents cooperating builds, tests, and profilers from overlapping timing; it does not exclude unrelated host activity. The workload is persistent IPv4 loopback TCP echo with payload validation, `TCP_NODELAY`, reusable buffers, and one request outstanding per connection. RTT includes client scheduling. Recorded CPU totals cover whole process lifetimes, including warmup and shutdown, rather than only the measurement window.

To rerun, build both sources into separate executables and invoke the runner with `--output DIR --build BUILD_JSON --client CLIENT --repeats 3`, ten `--case LABEL,BACKEND,BINARY` arguments (before/after for all five backends), and `--workload` arguments `1,1,64`, `1,32,64`, `1,32,4096`, `4,128,64`, `4,128,4096`. `BUILD_JSON` supplies source/build provenance; the runner records executable and script hashes itself.

Three short repetitions provide observed ranges, not statistical equivalence or a guaranteed regression bound. These results do not predict physical-network or general-application performance. Linux and macOS workspace checks passed with 100% measured source-region coverage, and CI also passed Windows coverage and Rust 1.85 compatibility for the measured source.
