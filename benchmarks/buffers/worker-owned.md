# Eager worker-owned receive pools

This is a historical experiment. Worker-provided pool construction and its receive-ahead integration have been removed from ordinary readers. Reproduce the comparisons at the recorded source revisions; current applications overlap reads and writes explicitly.

The measurements below describe the implementation at their recorded source revisions. The current portable `BufferPooling` interface exposes only `buffer_pool(options)`; the per-worker comparison is now a concrete `IoUringSelector::buffer_pool_per_worker(options)` experiment accessed through the Native scheduler's selector. Moving that control does not change the measured receive algorithms.

Worker-owned storage removes cross-worker buffer reclamation and makes receive capacity predictable per worker. Ordinary shared pools retain the simpler syscall-first monitor receive path. Across the longer equal-budget run, worker pools improve the multiworker aggregate by 11.4% over the previous shared provided-ring implementation, but remain 6.7% behind monitor receives and 14.6% behind the Tokio adapter. Keep receive-ahead explicit; local storage alone does not make it the universally faster default.

## Design

`buffer_pool(options)` retains an exact total payload budget and portable monitor receives. The concrete `IoUringSelector::buffer_pool_per_worker(options)` experiment explicitly reserves that many slots for every I/O worker, and Native callers access it through `scheduler.selector()`. At the measured revisions this experimental control was exposed on `BufferPooling`, forwarded by Native, and returned `Unsupported` on other backends; it has since been removed from that portable capability. `capacity()` reports total slots, including all workers.

For worker-owned io_uring pools, construction allocates every worker's contiguous payload slab, buffer-ID table, vacant-index list, and provided-ring storage before publishing initialization commands. Each driver registers its group on its next turn, including startup. A synchronous constructor never waits for workers that may not have started. Payload slabs and provided-ring storage require no first-use allocation; receiver bookkeeping and the reusable capacity listener can still allocate when first needed. Registration errors are cached for the affected shard and reported when preparing a receiver, without stopping unrelated scheduler work.

One selector-wide reusable 16-bit group ID identifies the pool on every ring. Reactor tables index directly by that ID; the pool indexes worker shards by stable worker index. Existing private TLS identifies the current worker. There is no per-pool TLS key or pool-to-worker hash map. The selector supports at most 65,536 live group reservations, including legacy explicit multishot resources.

Connections on a worker share its group. An admitted receiver stays with that ring if its consumer task migrates. Publication transfers exclusive leases into the worker's buffer-ID table; a receive completion transfers the selected lease to the application. Mutable chunks and frozen parser views can move between threads, but their last owner always returns storage directly to the original slab. This removes donor selection, cross-worker reclamation, and cancellation solely to redistribute buffers.

Local exhaustion backpressures even if another worker has capacity. Published storage belongs to accepted receives, including idle receives; use a separate ordinary pool for independent scratch buffers or file pipelines. Group IDs remain reserved until registrations retire, while already delivered leases retain their home slab independently of numeric IDs. Closing, dropping, or reusing the logical pool cannot redirect an old lease into a different pool. Unexpected driver failure retains storage whose kernel retirement cannot be established.

## Comparison and memory budgets

All network cases receive bursts of eight payloads after sending one request byte. `pool` releases each completed chunk; `shared` freezes and retains a parser view until a later receive. These are loopback transport exercises without HTTP framing, TLS, QUIC, physical-network traffic, or application parsing. The `tokio` strategy is the Core Tokio adapter, not bare Tokio.

| Strategy | Implementation | Payload capacity |
| --- | --- | --- |
| `shared-provided` | Previous shared-budget provided-ring implementation, including reclamation fixes, at `fd47abe0c0ba939e64ad1538cbc50e343064897c` | Eight slots per connection, total |
| `shared-monitor` | New ordinary shared pool and syscall-first monitor receiver | Same total |
| `worker-equal` | New worker-owned provided-buffer rings | Same total, divided equally between workers |
| `worker-scaled` | New worker-owned provided-buffer rings | The old total count on **each** worker |
| `tokio` | Core Tokio adapter and ordinary shared pool | Same total |

With one worker and one connection, every strategy has eight slots; there is no separate scaled sample. With two workers and eight connections, the equal-budget strategies have 64 total slots, and `worker-equal` reserves 32 on each worker. `worker-scaled` reserves 64 on each worker, or 128 total. For 64 KiB payloads this means 4 MiB versus 8 MiB of usable payload storage. Receive metadata, padding, publication rings, and runtime bookkeeping are additional memory, so equal payload capacity is not equal total resident memory.

All executables use Cargo's optimized release profile on Aiko's Ryzen 9 9950X3D, Linux 7.2.6-arch2-1, and Rust 1.99.0. No builds, tests, or other agent benchmarks overlap timing. Each connection warms 1,000 bursts. Case order and strategy order within each matched repeat are randomized with a fixed seed. Tables report the median of seven per-repeat throughput ratios, not the ratio of independently computed medians. Geometric means give each transport/size/retention case equal weight; they are descriptive point estimates, not confidence bounds.

### Initial comparison

The [initial dataset](results/worker-owned.jsonl) contains 378 samples: twelve cases, seven repeats, and four strategies for single-worker cases or five for multiworker cases. It measures 10,000 bursts per connection with CPU affinity 2–9. The candidate is `934edc5d6411ec4cadc1c6e94c32cc2ba580d49a`. Each row records the executable SHA-256, revision, arguments, strategy, repetition, and usable payload budget.

Equal-budget worker pools versus the previous shared provided-ring implementation, change in messages per second:

| Transport and payload | 1 worker / 1 connection, immediate | 1 worker / 1 connection, retained | 2 workers / 8 connections, immediate | 2 workers / 8 connections, retained |
| --- | ---: | ---: | ---: | ---: |
| TCP, 64 bytes | −0.2% | −0.3% | +16.2% | +0.2% |
| TCP, 64 KiB | +0.1% | +0.5% | −2.5% | +3.5% |
| UDP, 1,200 bytes | −0.6% | −0.3% | +0.5% | +22.2% |

The geometric mean is 1.030× the old provided-ring implementation, 0.930× the new shared-monitor strategy, and 0.948× the Tokio adapter. Single-worker results against the old implementation are essentially unchanged. Multiworker paired ratios vary widely, including −45.2% to +95.1% for small TCP without retained views. Eight peer threads plus two runtime workers share eight allowed physical cores. This run does not establish a reliable multiworker speedup. Doubling capacity gives a 1.018× geometric mean over the six multiworker cases, also with broad variation.

### Longer follow-up

The [follow-up dataset](results/worker-owned-long.jsonl) contains another 378 samples, using 50,000 measured bursts per connection and CPU affinity 2–15: fourteen physical cores for eight peer threads plus two runtime workers. This removes the initial run's CPU-count oversubscription, although it does not pin each thread or control placement within the CPU's cache domains. The candidate is `35ac2e952b6708890c1756b639ef8876f9489f0b`, recorded as `35ac2e9` in the rows. Compared with the first candidate, its runtime change isolates idle-group re-registration failures after successful cleanup; steady-state receive behavior is unchanged. The baseline remains `fd47abe0c0ba939e64ad1538cbc50e343064897c`.

Median changes in messages per second. The first three comparison columns use `worker-equal`; the final column measures the effect of doubling the worker pool's payload capacity:

| Workers / connections | Transport and payload | Retention | Versus old provided | Versus new monitor | Versus Tokio adapter | Scaled versus equal |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| 1 / 1 | TCP, 64 bytes | Immediate | −0.4% | +2.6% | −1.5% | — |
| 1 / 1 | TCP, 64 bytes | Retained | +0.1% | +3.4% | −0.5% | — |
| 1 / 1 | TCP, 64 KiB | Immediate | +0.0% | +3.7% | +1.9% | — |
| 1 / 1 | TCP, 64 KiB | Retained | +0.3% | +3.7% | +0.6% | — |
| 1 / 1 | UDP, 1,200 bytes | Immediate | +0.8% | +2.9% | −3.1% | — |
| 1 / 1 | UDP, 1,200 bytes | Retained | +2.2% | +5.1% | −2.6% | — |
| 2 / 8 | TCP, 64 bytes | Immediate | +6.4% | −3.4% | −12.8% | +0.2% |
| 2 / 8 | TCP, 64 bytes | Retained | +11.7% | −4.6% | −14.1% | +3.2% |
| 2 / 8 | TCP, 64 KiB | Immediate | +8.8% | −20.1% | −25.5% | +4.5% |
| 2 / 8 | TCP, 64 KiB | Retained | +16.7% | −15.1% | −19.4% | +3.8% |
| 2 / 8 | UDP, 1,200 bytes | Immediate | +17.6% | +2.4% | −7.8% | +6.4% |
| 2 / 8 | UDP, 1,200 bytes | Retained | +7.8% | +3.3% | −6.8% | +7.7% |

Across the six single-worker cases, the geometric means are 1.005× old provided, 1.036× monitor, and 0.991× the Tokio adapter. Across the six multiworker cases they are 1.114×, 0.933×, and 0.854× respectively. Across all twelve cases they are 1.058×, 0.983×, and 0.920×. Single-worker case medians against the Tokio adapter range from −3.1% to +1.9%; the larger multiworker gap remains.

The longer run has less extreme variation, but still does not isolate the costs responsible for the remaining gap. For example, multiworker 64 KiB TCP without retention is 20.1% behind monitors, with individual matched ratios from −33.1% to −13.3%. Against the old provided-ring implementation the same case has a +8.8% median but spans −10.6% to +25.5%. CPU count and sample duration changed together, so differences between runs cannot be attributed to either change alone. Result-queue synchronization, refill work, placement, task migration, and completion delivery remain candidates for profiling rather than established causes.

Doubling capacity improves the six multiworker cases by a 1.043× geometric mean; individual case medians range from +0.2% to +7.7%. It costs twice the usable payload memory, and several repeat ranges cross zero. This is not a free throughput gain or a reason to silently multiply ordinary shared-pool capacity.

### Lease control

The [lease-only control](results/worker-owned-leases.jsonl) contains 80 samples: five matched before/after repetitions of eight cases using the same baseline and final candidate release builds. It isolates ordinary heap-pool acquisition, optional freeze/view creation, and return, without networking or kernel registration. Affinity is 2–9; one thread performs 1,000,000 acquisitions, or eight threads perform 250,000 each, with either independent or shared pools.

The geometric mean across eight case medians is 1.001× baseline. Uncontended case medians range from +0.1% to +1.8%; the two contended eight-thread shared-pool cases are +4.7% mutable and −8.8% frozen, with wide repeat ranges. The contiguous slab introduces no clear aggregate lease-throughput regression, but these short, noisy samples do not prove equivalent contended performance. Allocation time and page residency are outside this timed loop.

## Recommendation

Keep ordinary exact-budget shared pools and monitor receives as the portable default. Keep eagerly provisioned worker pools as an explicit io_uring receive-ahead policy: they have simpler ownership and retirement than cross-worker reclamation, predictable local capacity, and a measurable aggregate improvement over that earlier design. They do not yet meet the goal of consistently exceeding monitor or Tokio-adapter throughput, especially for multiworker TCP. Profile that path before promoting it as the faster default. Fixed-buffer file registration and zero-copy sends remain separate experiments; provided-buffer receive still copies data from kernel to userspace.

The subsequent [receive-cost investigation](receive-cost.md) identifies substantial buffer-locality costs and persistent ring load imbalance in the large-TCP case. Controlled buffer rotation reproduces much of the regression without provided-ring return machinery; balanced four-worker receive placement can exceed the matching monitor control. These diagnostic results refine the explanation without changing the default recommendation or adding a production optimization.

## Validation

Linux covers 31,257/31,257 measured source regions, excluding 11 invariant-only unreachable regions. macOS covers 22,078/22,078, excluding four. These are full-workspace, all-target, all-feature coverage checks. Clippy passes on both platforms. [CI on the final implementation commit](https://github.com/core-rs/core-scheduler/actions/runs/39736685573) also passes the Linux, macOS, and Windows coverage jobs and Rust 1.85 compatibility tests. The final Linux check includes isolated idle-group re-registration failures; that correction changes error routing after successful cleanup, rather than timed receive behavior.

Miri passes the 14 applicable public buffer-pool tests, including simultaneously live views into distinct contiguous slots and cross-thread returns. Leak checking is disabled because existing tests intentionally forget a lease to verify safe leakage; aliasing checks remain enabled. Tests also exercise up-front initialization before workers start, local exhaustion, dormant-receiver wakeups, registration failures, group-ID reuse, alignment, retained payloads, cancellation, and teardown. Measured coverage is not proof of kernel behavior on every supported Linux release.

## Reproduce

Build the baseline and candidate separately with `cargo build --release --locked -p core-scheduler-buffer-benchmark --bins`. On an otherwise idle Linux host with provided-buffer ring and multishot receive support:

```sh
flock /tmp/core-scheduler-perf.lock \
  python3 benchmarks/buffers/worker_owned.py \
  --baseline /path/to/baseline/release --candidate /path/to/candidate/release \
  --candidate-revision COMMIT --repeats 7 --iterations 10000 --cpus 2-9 \
  --output /tmp/worker-owned.jsonl
python3 benchmarks/buffers/summarize_worker_owned.py /tmp/worker-owned.jsonl \
  --output /tmp/worker-owned-summary.json
```

For the longer run, use `--iterations 50000 --cpus 2-15`. The runner expects the `sockets` executable in both directories. Do not overlap builds, coverage, or other benchmarks. The lease control uses `pool_contract.py --before /path/to/baseline/release --after /path/to/candidate/release --repeats 5 --match '^leases ' --output /tmp/worker-owned-leases.jsonl`; summarize it with `summarize_pool_contract.py`. That runner expects all three benchmark executables (`leases`, `sockets`, and `files`) even when selecting only lease cases.

The older [shared-budget experiment](worker-pools.md) remains available as historical research.
