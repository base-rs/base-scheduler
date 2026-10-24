# Native loop-order controls

All candidates preserve production commit `49202e58690c795f63985cd0acddab460a05387c` except the explicitly named control. Each uses a separate local and Aiko worktree. Builds and timings hold `/tmp/core-scheduler-perf.lock`, changed Rust files are touched before compilation, and source/binary hashes are retained. Builds use release optimizations and `CARGO_PROFILE_RELEASE_DEBUG=1`; client binary is fixed. Server CPUs are 0–3, client CPUs 4–7, with 1-second warmup and 3 measured seconds. All cases transfer 64 bytes. Each table reports medians from three randomized repetitions.

## Restore the post-batch driver turn

Before is frozen pre-batch candidate `49202e5` (driver turn before runnable batch). After changes only `crates/native/src/worker.rs` back to the `1840258` order: runnable batch, then nonblocking driver turn. Guard, borrowed controller routing, cached owned receives and AtomicWaker retirement signaling are identical. There are 45 samples including raw Tokio controls.

| Backend | Workers | Connections | Before req/s | After req/s | Change | Before P99 µs | After P99 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| iouring-monitor | 4 | 1 | 263,754 | 261,326 | -0.92% | 5.603 | 5.611 |
| iouring | 4 | 1 | 196,643 | 229,440 | +16.68% | 7.403 | 7.391 |
| iouring-monitor | 1 | 1 | 272,943 | 273,312 | +0.14% | 4.383 | 4.291 |
| iouring | 1 | 1 | 244,333 | 240,470 | -1.58% | 5.991 | 5.743 |
| iouring-monitor | 1 | 32 | 630,169 | 634,144 | +0.63% | 56.575 | 55.007 |
| iouring | 1 | 32 | 658,544 | 650,143 | -1.28% | 52.863 | 54.591 |

Owned 4-worker/1-connection samples establish a substantial loop-order effect: current pre-batch ranges 193,726–196,736 req/s; original post-batch ranges 228,645–236,923 req/s. Most other differences are small and ranges overlap. Borrowed 4-worker/1-connection has one low post-batch outlier, 239,735 req/s, versus the other two 261,326 and 263,939; its median remains close to pre-batch candidate. Do not infer the single-connection P99 from throughput: these measure different parts of the distribution.

## Restore Event retirement signaling

Both sides retain the original post-batch loop order. Before retains AtomicWaker; after restores only `operation_state.rs` to `1840258` and removes its new AtomicWaker-specific test module. There are 18 samples, including raw Tokio controls.

| Backend | Workers | Connections | Before req/s | After req/s | Change | Before P99 µs | After P99 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| iouring | 4 | 1 | 234,596 | 236,451 | +0.79% | 6.591 | 6.111 |
| iouring | 1 | 32 | 646,804 | 643,698 | -0.48% | 53.631 | 53.695 |

No meaningful throughput improvement is established. Keep AtomicWaker rather than expanding this work into additional retirement signaling changes.

## Avoid a second driver turn after idle completion

Before is frozen pre-batch candidate `49202e5`. After adds a worker-local boolean tracking whether the idle path has already serviced the driver since the previous runnable batch. It skips the otherwise redundant nonblocking pre-batch turn after an idle completion. Busy batches still receive a driver turn between batches. No other production changes are made. There are 45 samples including raw Tokio controls.

| Backend | Workers | Connections | Before req/s | After req/s | Change | Before P99 µs | After P99 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| iouring-monitor | 4 | 1 | 270,343 | 273,610 | +1.21% | 5.243 | 5.083 |
| iouring | 4 | 1 | 194,078 | 195,973 | +0.98% | 9.751 | 9.671 |
| iouring-monitor | 1 | 1 | 270,174 | 272,915 | +1.01% | 4.311 | 4.311 |
| iouring | 1 | 1 | 237,123 | 238,957 | +0.77% | 5.903 | 5.903 |
| iouring-monitor | 1 | 32 | 628,551 | 629,534 | +0.16% | 55.423 | 55.359 |
| iouring | 1 | 32 | 657,513 | 648,257 | -1.41% | 53.151 | 53.471 |

The flag does not recover the substantial owned 4-worker/1-connection difference. Its candidate range is 192,272–197,091 req/s, overlapping the 192,140–194,649 pre-batch candidate range. It fixes a real redundant driver turn, but that turn does not explain the loop-order regression. Prefer restoring the original post-batch loop and omitting the flag, as validated in [the final comparison](../../../../tokio-parity.md).

The candidate and regression test are committed locally as `6cb7f4ea9151b5dedb13624c74996713f10b5a47` on `perf/native-loop-serviced`. This is a diagnostic candidate, not a recommendation to integrate its boolean. The fake driver records the turn that publishes a completion; the resumed consumer must observe that same turn count. `regression-before.log` demonstrates failure on unmodified production `49202e5`; `serviced-tests.log` shows the assertion and complete Native test suite passing with the candidate, including driver fairness, timer and remote-wake tests. The original post-batch order also provides the intended critical-path behavior.

## Artifacts

`loop-control.json`, `state-control.json`, and `serviced.json` preserve every timing, P99 and executable hash. Matching `*-source.txt` files preserve commit, source hashes and source diff hashes. `loop-restored.patch`, `event-control.patch`, and `serviced.patch` preserve the exact changes; `compare.py` is the fixed runner.
