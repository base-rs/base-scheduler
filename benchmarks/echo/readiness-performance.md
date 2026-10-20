# Cached readiness and cooperative completions

This comparison measures established-stream Tokio readiness and fresh thread-bound io_uring cooperative completion processing against merged PR #13. The final optimized before/after matrix contains 150 samples on Aiko, covering all five backends and five workloads. [Raw samples](results/readiness-performance/final/aiko.json), [build and host provenance](results/readiness-performance/final/provenance.json), and the [validated summary](results/readiness-performance/final/summary.json) preserve the evidence separately from exploratory runs.

## Why the Tokio adapter made extra receives

The original adapter always attempted a receive before consulting readiness. After a previous successful read, Tokio could also retain a readable event. The next call therefore tried the syscall twice before suspending: once speculatively, then once through the stale readiness guard. TLS counters observed approximately two `WouldBlock` results per successful receive in the concurrent echo workload.

Nonempty scalar reads now consult existing cached readiness first after a stream has successfully completed a nonempty read. Lifecycle operations, empty reads, datagrams, and sockets without that established state retain their direct syscall behavior. Both reserved polling waiters and independent concurrent waiters learn the stream state. No payload pointer survives a synchronous syscall; ownership and cancellation contracts remain unchanged.

The [selected-policy counters](results/readiness-performance/explorations/tokio/safe-counters/samples/aiko.json) show one failed receive per steady read instead of two: 2,593,017 retry failures for 2,593,094 successful owned 64-byte reads, with just 64 initial failures. Borrowed reads and 4 KiB payloads show the same pattern. Instrumented runs establish the mechanism; their throughput is not used as a capacity result.

### Short reads and TCP urgent data

An earlier prototype also cleared readable events after short reads. That reached approximately 97% of pure Tokio in its focused pilot, but a short TCP read can stop at an urgent-data boundary while ordinary bytes remain queued. Clearing readiness there can stall the next read because another edge need not arrive. The [regression](results/readiness-performance/explorations/urgent-data/test.rs) failed on the prototype and passed with only this clearing removed. Production retains readable events until `WouldBlock`, including after short reads.

A [separate raw Tokio 1.53.3 test](results/readiness-performance/explorations/raw-tokio-urgent/test.rs) reproduced the same boundary: `AsyncRead::poll_read` obtained the prefix, then timed out for 100 ms, while a direct receive immediately obtained the queued suffix. Its independent `readable()` plus `try_read` control obtained both payloads. This is observed behavior of the tested version and configuration, not a claim about every Tokio version or workload. The normal pure-Tokio echo control uses `AsyncReadExt::read`, so its short-read readiness policy differs from Core's conservative policy. The rejected prototype and its incomplete integration matrix remain labelled as exploratory evidence.

### Comparing the same readiness policy

A separate [15-sample diagnostic](results/readiness-performance/explorations/readiness-policy/analysis.md) changes only the pure-Tokio receive loop, preserving `write_all`, the optimized build settings, and the one-worker/32-connection/64-byte workload. The polling variant uses `poll_fn(TcpStream::poll_read_ready)` followed by `try_read`, retaining readiness until `WouldBlock` as Core does. The async-waiter variant is a separate control because its waiter path differs.

| Receive loop | Median requests/s | Relative to Tokio `AsyncRead` | Relative to Tokio polling |
| --- | ---: | ---: | ---: |
| Tokio `AsyncRead` | 696,293 | 100.00% | 105.27% |
| Tokio `readable` + `try_read` | 649,123 | 93.23% | 98.14% |
| Tokio `poll_read_ready` + `try_read` | 661,454 | 95.00% | 100.00% |
| Adapter owned | 641,955 | 92.20% | 97.05% |
| Adapter borrowed | 644,560 | 92.57% | 97.45% |

The adapter is within 3% of Tokio using a comparable polling policy in this workload. The raw loop's 5% difference from `AsyncRead` includes readiness API, buffer handling, and cooperative-budget differences as well as receive-attempt behavior; this experiment does not isolate the cost of one `EAGAIN`. It narrows the unexplained wrapper overhead without replacing the original pure-Tokio comparison or establishing universal parity.

## Cooperative io_uring task work

Fresh thread-bound workers request `COOP_TASKRUN` and `TASKRUN_FLAG`, reducing kernel completion interrupts when the worker can process task work at its ring boundary. Existing task-run checks enter the kernel when needed. The worker publishes completion batches to ready queues before running tasks as before.

Cooperative task work belongs to its submitting thread. Generic movable drivers therefore keep the original ring configuration. Only a never-driven ring may be replaced when it enters its thread-bound worker; queued controller requests have not reached that ring yet. Already-driven and retired drivers are preserved, and every optional setup error retains the original validated ring. Tests exercise successful replacement with queued I/O, setup failures, pending work on a started ring, and movable-driver progress across live threads.

## Final comparison

With four workers, Native reaches 97.4–98.4% of pure Tokio and the adapter reaches 95.5–97.2%. Native improves 3.2–4.5% over the preceding implementation in these concurrent cases. With one worker and 32 connections, the adapter improves 3.4–4.7%, reaching 92.1–94.2% of pure Tokio; the original 5% target is still missed there. Native remains behind pure Tokio in every workload.

The single-connection borrowed Native median is 4.2% lower in this matrix, with overlapping before/after ranges. A separate [15-sample follow-up](results/readiness-performance/explorations/native-serial/analysis.md), with five repetitions per candidate, measured a 2.61% median and 1.46% mean reduction. Removing only the cooperative flags recovered 1.56% by median but 0.32% by mean; all ranges overlapped substantially. A small serial tradeoff remains plausible, but these samples do not establish the original 4.2% gap as the cost of the flags. The selected implementation retains cooperative processing without adding a workload-dependent policy.

| Workers/connections/bytes | After Tokio requests/s | Tokio owned | Tokio borrowed | Native owned | Native borrowed |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1/1/64 | 269,126 | 100.1% | 99.8% | 86.3% | 84.5% |
| 1/32/64 | 695,268 | 92.1% | 92.6% | 90.4% | 81.4% |
| 1/32/4096 | 611,452 | 93.0% | 94.2% | 91.0% | 83.8% |
| 4/128/64 | 2,107,227 | 95.5% | 96.3% | 98.1% | 97.4% |
| 4/128/4096 | 1,857,401 | 96.6% | 97.2% | 97.7% | 98.4% |

## Throughput change from before

| Workers/connections/bytes | Pure Tokio control | Tokio owned | Tokio borrowed | Native owned | Native borrowed |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1/1/64 | -0.4% | +0.5% | +0.3% | +0.8% | -4.2% |
| 1/32/64 | +0.2% | +4.7% | +3.8% | +0.3% | +0.7% |
| 1/32/4096 | -1.0% | +3.4% | +4.3% | -1.2% | -0.0% |
| 4/128/64 | -0.2% | +1.4% | +1.6% | +4.4% | +3.2% |
| 4/128/4096 | -0.0% | +0.3% | +1.7% | +4.5% | +3.7% |

The complete [table of before/after ranges](results/readiness-performance/final/tables.md) and [latency summary](results/readiness-performance/final/summary.json) retain all repetitions. Pure-Tokio controls use each corresponding optimized executable. Ratios compare medians within the same build and workload; before/after changes compare the two medians directly.

## Experiments retained as evidence

The [exploration index](results/readiness-performance/readme.md) distinguishes selected candidates, diagnostics, and rejected implementations. In particular:

- Bypassing the owning task wrapper improved the old adapter by only 0.3–0.6%; its ownership and panic handling remain intact.
- Explicit inlining produced no useful improvement: approximately unchanged owned throughput and slightly lower borrowed throughput in its pilot.
- Replacing the native readiness reply channel, shortcutting single-ring routing, and simplifying single-worker wakes did not produce enough benefit to retain.
- Adding a speculative synchronous receive before owned io_uring reads regressed the tested workloads. Kernel receive poll-first also regressed the concurrent single-worker case.

The native profiles still identify per-wait request admission, allocation, completion replies, and one-shot ring readiness registration as significant work. Persistent readiness could remove that work, but requires a separate resource-lifetime and cancellation design. These measurements do not establish the benefit of such a design.

## Reproduction and limits

Workspace checks pass on Linux and macOS, including 100% measured source-region coverage. The [verification record](results/readiness-performance/final/validation.md) includes logs, the tested source revisions, and the Linux conformance-fixture memory adjustment.

Before source is `427be5d2ef6c610a5d5de5b10570c938b65d6c2d`; selected source is `958ac6cf9862fdce2fc8de18c8864c4f971b6abc`. Both builds use the locked dependencies and default optimized Cargo release profile, with no explicit LTO or target-CPU override. The [provenance](results/readiness-performance/final/provenance.json) records both executable hashes, source patch, compiler, client, runner, CPU topology, and governor settings.

Each cell runs three randomized repetitions with one second of warmup and three seconds of measurement. Server workers use CPU 0 or CPUs 0–3; the fixed four-worker client uses physical cores 4–7 in the same last-level cache. Timings hold an exclusive performance lock, while builds and tests use its shared mode. The canonical user checkout on Aiko is unchanged.

This is persistent IPv4 loopback TCP echo, with `TCP_NODELAY`, a 64 KiB reusable server buffer, payload validation, and one outstanding request per connection. Throughput counts complete round trips entirely inside the measurement window. RTT includes client scheduling. CPU values cover server lifecycle, including setup and warmup. The results measure these concrete implementations, ownership paths, and readiness policies together; they do not isolate trait dispatch or predict physical-network or general application performance. Three repetitions provide a focused comparison, without a statistical guarantee at one-percent precision.
