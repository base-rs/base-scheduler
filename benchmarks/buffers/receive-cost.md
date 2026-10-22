# Why provided-buffer receives cost more

The largest TCP regression is substantially explained by buffer reuse and cache locality, with persistent ring load imbalance making some runs worse. It is not adequately explained by the cost of returning a lease. Ordinary monitor receives can repeatedly overwrite a recently used buffer; eager multishot receives rotate through a larger working set and deliver completed leases through the reactor. These differences matter even though both paths use reusable pools and receive the same bytes.

The evidence below comes from 260 additional optimized samples on Aiko, plus exploratory profiles. These are loopback receive-only exercises, not HTTP, TLS, physical-network, or application-parser results. No production scheduler optimization is included in this investigation. The diagnostic implementations are preserved as patches, rather than added to the runtime.

## Cache locality: a controlled intervention

The ordinary heap pool normally acquires the lowest available slot. With immediate release, a connection can reuse that slot on its next receive. The provided-buffer ring instead rotates through published buffers. At two workers and eight connections, its equal-budget pool publishes 32 buffers per worker: slightly more than 2 MiB including metadata for 64 KiB payloads. Aiko has 1 MiB of L2 cache per physical core. By comparison, a single-connection eight-buffer working set is about 512 KiB.

To test this difference, the monitor control was changed to rotate its acquisitions through the pool, retaining its ordinary direct `recvmsg` implementation. A second control performs the same additional atomic cursor update but still acquires the lowest available slot. Each strategy uses the same executable, two workers, eight connections, bursts of eight 64 KiB payloads, and 64 total payload slots. Every sample receives 125,829,120,000 measured bytes after warmup.

The [cursor-control dataset](results/receive-cost-cursor.jsonl) has seven randomized matched repetitions. Throughput is in logical payloads per second; percentages are medians of per-repeat ratios against the monitor, rather than ratios of the independently computed medians.

| Strategy | Median payloads/s | Change versus monitor | Matched range |
| --- | ---: | ---: | ---: |
| Monitor, normal reuse | 1,042,973 | — | — |
| Monitor, extra atomic cursor only | 1,029,283 | −0.6% | −2.4% to +1.4% |
| Monitor, rotating buffers | 897,954 | −13.7% | −16.3% to −8.5% |
| Provided-buffer multishot | 892,332 | −14.7% | −35.5% to −11.0% |

Changing reuse policy alone reproduces a substantial slowdown without adding kernel receive operations, completion queues, provided-ring publication, or lease-return machinery. The cursor-only control argues against its additional shared atomic being the main explanation. This does not establish an exact additive partition of the provided path's costs.

### Hardware-counter corroboration

A separate [five-repeat counter run](results/receive-cost-cache-stat-complete.jsonl) attaches `perf stat` to the two native worker threads, excluding the eight explicitly named peer threads. It uses 60,000 bursts per connection, twice the measured bytes of the other runs. Counts include the observed startup/warmup and cleanup; attachment shortly after worker creation misses a small initial interval. Hardware events are multiplexed at approximately 83% and scaled by `perf`.

| Strategy | Median payloads/s | Change versus monitor | Worker instructions, billions | Worker L2 misses, billions |
| --- | ---: | ---: | ---: | ---: |
| Monitor, normal reuse | 1,050,550 | — | 35.788 | 1.442 |
| Monitor, rotating buffers | 944,462 | −10.1% | 35.816 | 4.404 |
| Provided-buffer multishot | 894,138 | −14.9% | 42.168 | 4.391 |

The rotating monitor executes essentially the same number of instructions but incurs about three times as many L2 misses. The provided path has a similar L2-miss count and also executes more instructions. The L2 event is `l2_cache_req_stat.ic_dc_miss_in_l2`; it includes instruction and data requests. Together with the reuse intervention, this is strong evidence for a substantial memory-locality cost. It does not imply that the remaining difference is exclusively lease return or that every cache miss has the same latency.

An earlier [cache experiment](results/receive-cost-cache.jsonl) also measured 8 KiB payloads, smaller and larger publication budgets, and worker task polls. Rotating the monitor cost 11.1% at 64 KiB and 4.1% at 8 KiB. These sizes and worker counts change several effects together; they are supporting evidence rather than a clean measurement of an L2-capacity threshold.

## Persistent receives can leave a worker overloaded

A receiver chooses the current worker's ring when constructed and stays there. Consumer tasks can migrate, but stealing a consumer does not move the kernel receive or its payload-copy work. Initial task placement sometimes assigns six of eight receivers to one ring and two to the other. This creates an imbalance that consumer work stealing cannot directly repair.

The [placement experiment](results/receive-cost-balance.jsonl) records each receiver's ring owner and compares the normal route with an experimental round-robin assignment at construction. Payload capacity stays at 64 slots across all worker counts. Every row compares against a monitor with the **same** worker count; each has seven randomized matched repetitions.

| Workers | Provided receive placement | Median payloads/s | Change versus matching monitor | Matched range |
| --- | --- | ---: | ---: | ---: |
| 1 | All eight on the only ring | 524,905 | −6.6% | −9.5% to −4.3% |
| 2 | Current worker | 788,805 | −25.5% | −33.3% to −12.9% |
| 2 | Four receivers per ring | 895,346 | −14.9% | −18.3% to −13.3% |
| 4 | Current worker | 1,052,287 | −0.6% | −20.9% to +8.0% |
| 4 | Two receivers per ring | 1,176,472 | +9.0% | −2.2% to +19.1% |

In the two-worker normal-route samples, all three 4/4 assignments reach roughly 898–905 thousand payloads/s. The 6/2 assignments reach roughly 691–739 thousand. Round-robin makes the connection count even and removes that source of variation. It does not prove equivalent byte or CPU load for a real server with heterogeneous connections, and routing away from the constructing worker can introduce remote coordination. It is a diagnostic policy, not a proposed universal scheduler change.

The four-worker result also demonstrates that provided-buffer receive is not intrinsically slower. More workers change available CPU resources and per-worker working sets together. Its advantage here should not be attributed solely to cache size or extrapolated to all workloads.

## Receive fragmentation and exhaustion add work too

The [topology control](results/receive-cost-topology-control.jsonl) compares one or eight connections on two workers. At eight connections, the monitor's median actual receive count is about 1.006 per logical 64 KiB payload, versus 1.065 for equal-budget provided receives. TCP does not preserve the benchmark's payload boundaries. Eager receives can consume smaller available chunks, and each chunk becomes a delivered lease and completion.

The 32-slot worker rings also run out of provided buffers frequently. In this run, doubling capacity from 32 to 64 slots per worker reduces median exhaustion events from 145,231 to 6,108, but changes the paired median gap against monitors only from −13.3% to −11.0%. Statistics include the 1,000 warmup bursts; byte and receive-call counters exclude them. More capacity reduces rearming without removing the other costs.

Shrinking the pool is not a sufficient cache optimization either. In the cache experiment, reducing total capacity from 64 to 16 slots makes the provided path 30.8% slower than the monitor, compared with 14.6% at 64 slots. Median task polls increase from 368,239 to 666,987. Conversely, increasing capacity to 256 slots reduces polls to 259,079 but leaves the throughput gap at 14.9%. Publication depth, reuse, starvation, and delivery batching need to be considered together.

The [isolation experiment](results/receive-cost-isolation.jsonl) provides further negative controls:

- The older private, per-connection multishot pools are 21.8% behind monitors, essentially the same as the worker-pool path in that run. They have eight buffers per connection, the same 64-slot total, but a different partition and return implementation. This argues against the new shared worker-pool refill machinery being the sole or dominant explanation; it does not isolate the cost of every return instruction.
- Replacing TCP `RecvMsgMulti` with `RecvMulti` gives no clear recovery: the respective median gaps are 21.8% and 21.3%. Removing recvmsg metadata processing alone does not solve the regression.
- Page-aligned buffers do not consistently close the gap. The combined raw-receive/alignment variant also remains slower.
- With one-payload bursts, equal-budget provided receives have essentially equal paired median throughput to monitors in this run, although individual ratios span −20.8% to +8.6%. The burst-eight regression is not a universal per-operation tax.

Exploratory whole-process profiles put most CPU samples in kernel TCP and memory-copy work, including the peer threads. Individual buffer-return symbols have small self-sample shares, but inlining and downstream wakeups prevent treating those shares as a complete return-cost measurement. Tracepoint counting also perturbs the relative throughput; those timings are not used in the comparisons above.

## Design implication

Retain the portable ownership and shared-pool interface. The new evidence supports tuning the receive-ahead implementation rather than rejecting pools or assuming provided buffers eliminate receive-copy costs. A useful next optimization should preserve hot-buffer reuse where possible, avoid excessive publication and exhaustion, and account for the ring owner carrying persistent I/O work after consumer migration.

Keep ordinary monitor receives as the established portable fast path while evaluating those changes. Keep worker-owned multishot as explicit receive-ahead capacity, particularly where concurrent reception or retained parser views make that capacity useful. A real parser that retains and scans many chunks can change both paths' working sets; these immediate-release controls intentionally make reuse differences visible. No new performance claim about retained parsing, UDP, file registration, or zero-copy sends follows from this TCP experiment.

## Provenance and reproduction

All binaries derive from `c461fe8ff281d6d694c390d0f97d0096df730534`, built with `cargo build --release --locked -p core-scheduler-buffer-benchmark --bin sockets` and `CARGO_PROFILE_RELEASE_DEBUG=1`. Debug symbols are enabled; optimization remains the release profile. Aiko runs Linux 7.2.6-arch2-1 and Rust 1.99.0 on a Ryzen 9 9950X3D. Affinity is CPUs 2–15, fourteen physical cores spanning both L3 domains. Individual threads are not pinned, so cache-domain placement remains uncontrolled. Builds, tests, and timing runs are serialized using `/tmp/core-scheduler-perf.lock`.

The [provenance manifest](experiments/receive-cost/provenance.json) maps each dataset to its diagnostic patch, executable SHA-256, case-order seed, and sample count. Apply exactly one patch to a clean worktree at the base commit. Patches are cumulative alternatives, not a sequence to apply together. They add research-only environment controls and logging; they are not tested portable runtime changes. In particular, the raw multishot receive cap is exercised only on Aiko's kernel.

The isolation, cache, placement, and cursor datasets record exact arguments and environment overrides per sample. Replace the recorded executable path with a newly built binary. For example, after applying `cursor.patch`:

```sh
flock /tmp/core-scheduler-perf.lock \
  taskset -c 2-15 /path/to/sockets iouring pool 65536 30000 2 tcp 8 receive 8 total
flock /tmp/core-scheduler-perf.lock \
  env POOL_CURSOR=1 taskset -c 2-15 /path/to/sockets iouring pool 65536 30000 2 tcp 8 receive 8 total
flock /tmp/core-scheduler-perf.lock \
  env POOL_ROTATE=1 taskset -c 2-15 /path/to/sockets iouring pool 65536 30000 2 tcp 8 receive 8 total
flock /tmp/core-scheduler-perf.lock \
  taskset -c 2-15 /path/to/sockets iouring pool 65536 30000 2 tcp 8 receive 8 worker-equal
```

Every connection warms up for 1,000 bursts. Most samples measure 30,000 bursts per connection; the hardware-counter run measures 60,000. The topology control always uses burst size eight and varies connection count between one and eight; it does not vary burst size. The isolation experiment explicitly varies burst size at a fixed eight connections. Shuffle the case list independently within each repeat using the manifest seed. Pair ratios by `repeat` within a dataset and matching worker/size/burst topology. Different datasets use different experimental binaries, so their absolute medians should not be combined into one additive cost breakdown. Ranges are observed matched-repeat ranges, not confidence intervals.
