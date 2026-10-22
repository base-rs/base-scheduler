# Persistent ring state

Persistent state has two distinct uses: retaining an operation until it retires, and keeping reusable registrations between operations. The prototype gives both an explicit ring owner, adds a passive per-ring file cache, and preserves existing socket, timer, and buffer ownership contracts. This is a measured file optimization and a common migration vocabulary, not a claim that every kind of persistent state should migrate identically.

## Ownership and migration

`RingBinding<T>` pairs backend state with its owning controller. A controller is retained for as long as its identifiers or registration slots are meaningful. The explicitly supplied scheduler validates resource compatibility; TLS selects a compatible worker only for new work.

- **Active work stays on its original ring.** Moving the future does not move the operation. Completion, cancellation, and retirement use that binding. A later operation may choose the new worker after the old operation retires.
- **Passive registrations can be cached on visited rings.** `RingCache<T>` keeps the current entry inline and allocates its previous-entry list only after migration. Same-worker access checks one identity. Returning to a visited worker reuses its registration. Migration lookup is linear in the number of visited rings, bounded by the selector's worker set.
- **Some persistent state is itself active.** A socket monitor's multishot readiness poll is an outstanding kernel request. The existing migration policy retires that poll, including issued cancellation completions, before rebinding when an uncached wait needs another worker. It is not a passive cache entry. A ready syscall fast path does not require rebinding.
- **Release belongs to the original ring.** File monitor close first retires accepted I/O, queues release on every visited ring, and awaits acknowledgments. Cancelling close stops waiting while release continues; retry observes persistent results. Drop queues release without waiting. Drivers must remain driven during explicit close.

This does not change task placement, stealing, or pinning. Timers retain their existing worker-owned registration; provided-buffer receivers retain their original pool/ring. Those states do not silently become passive per-ring caches.

## Fixed files

Each file monitor retains a native file owner and caches a fixed-file lease on each visited ring. First use on that ring lazily creates a sparse kernel file table and registers the file. Later SQEs use the fixed-file index. `IoUringOptions::fixed_files` bounds the table per ring, defaults to 256, and can be set to zero. Unsupported table setup, insertion errors, or exhaustion retain the ordinary-descriptor path for that lease; an operation does not fail just because the optimization is unavailable. A fallback lease is not retried automatically when another slot becomes free.

There is setup and lifetime cost: each visited ring needs lease and retirement storage, registration/removal syscalls, and a retained file owner. Even with fixed files disabled, the present implementation retains the same lease/cache structure. Every operation still makes a fresh request identifier and acquires bounded transient capacity. Idle monitors do not hold transient-operation permits. The file table's cold-path mutex is not acquired for warmed submission. Slots are reused only after successful kernel removal; failed removal preserves its error and leaves the slot unavailable until ring destruction.

The public monitor's retained file is checked against its registration before dispatch or close. Safe public registration manipulation cannot redirect I/O to another file. This uses native-owner identity without cloning an `Arc`, allocating, or making a syscall.

Typed request boxes and completion slots were already reusable in the preceding prototype. They remain reusable through ring changes; tests alternate two rings and measure zero allocations after both registrations warm. A future that first submits externally and is subsequently polled by a different worker keeps its original active binding. The next operation can select the new worker.

## Other optimization opportunities

| State | Existing reuse or proposed opportunity | Decision and constraint |
|---|---|---|
| File request and completion | Reuse typed request and completion storage. | Implemented in the original monitor prototype; changing buffer-owner type replaces the request. Fresh identifiers protect delayed cancellation. |
| Fixed-file registration | Cache passive registration on each visited ring. | Implemented here; bounded table and ordinary-descriptor fallback. Measure incremental benefit separately from request reuse. |
| Socket readiness | Retain directional multishot poll and cached readiness. | Already implemented. Registering the socket as a fixed file could only help poll setup/rearm; direct recv/send syscalls still need the ordinary descriptor. No measured reason to add this lease complexity yet. |
| Timers | Retain worker-local registration and update deadlines. | Existing reusable timers already do this. Earlier wake/rearm can avoid queue updates when extending deadlines. Migration preserves the timer owner rather than moving an armed entry. |
| Fixed buffers | Avoid repeating kernel page lookup/pinning for suitable file buffers. | Not implemented or measured. Needs independently retained backing storage and safe leases; caching arbitrary returned `IoBuf` addresses is invalid. |
| Provided-buffer multishot | Reuse kernel-provided receive buffers and keep leases during parsing. | Already implemented with separate pool ownership. Remote lease return routes to the original ring; it must not publish into the current thread's ring. A future shared allocator can feed separate ring registrations. |
| Message vectors | Retain owner-safe, prepared I/O vectors. | Existing `IoBuffers` and `IoBuffersMut` already provide reusable vectors and advancement without payload copies. Another monitor-side vector cache needs evidence. |
| Completion dispatch | Replace numeric lookup with reusable slots or stable request pointers. | Candidate for profiling. Requires generation/ABA protection and both original/cancellation CQE retirement before storage reuse. A raw pointer alone is insufficient. |
| Completion transfer | Remove the mutex that transfers the completed buffer owner. | Candidate only if profiling justifies it. Current ready flag and atomic waker already avoid completion locking while pending. Lock-free storage would add unsafe synchronization. |
| Admission | Reuse capacity across sequential operations. | Do not retain scarce transient permits in idle monitors: enough idle resources could starve unrelated operations. File-registration capacity is separate. |
| SQE construction | Cache invariant opcode fields. | Unmeasured. The existing builders may already optimize to a few stores; pointer, length, offset, owner, and request identity can change each call. |
| Migration cache size | Retain only a bounded subset of visited rings. | Current cache is bounded by worker count and avoids registration churn. Consider eviction only if measured memory pressure warrants asynchronous release complexity. |
| Short-lived monitors | Delay fixed-file registration until reuse is established. | Unimplemented candidate. Current first-use registration is amortized by these long-running measurements; single-operation setup cost needs a separate workload. |

Registered-buffer storage must remain owned independently of application leases for the full registration lifetime. Generic `IoBuf` only promises stability while an operation owns it; once returned, its caller can resize or destroy it. A reusable pool can satisfy the stronger contract and allow completed data to be parsed on another worker, but address caching cannot. Such a pool also needs an explicit policy for registration on visited rings and asynchronous release. The current file optimization does not add a public pool API or claim fixed-buffer gains.

## Measurement method

The experiment compares three optimized binaries/variants:

- `registered`: current monitor implementation, default fixed-file capacity.
- `unregistered`: the same executable with `fixed_files = 0`, retaining the same cache and request code. This isolates incremental fixed-file benefit.
- `original`: the earlier file-monitor executable, before per-ring lease/cache changes. This exposes net benefit and the added bookkeeping cost.

Local cases use Native's executor with one task/one worker or 16 tasks/four workers. External cases use one caller and one or two standalone ring-driver threads; two drivers deliberately alternate rings on every operation. These include remote queue and wakeup costs, not executor work stealing. Warmup visits every ring before timing. Local and external timing boundaries differ; compare variants within each configuration, not their absolute rates as identical workloads. See the [benchmark instructions](readme.md) for exact boundaries.

Each case has seven randomized matched repetitions, 50,000 iterations per task, page-cache-hot tmpfs data, and 4 KiB or 64 KiB transfers at offset zero. Concurrent writes overlap the same file range. All processes are pinned: CPU 2 for one worker, CPUs 0–7 otherwise. Runs are serialized, with no concurrent builds or tests. Throughput changes are medians of paired ratios; reported ranges are observed minima/maxima, not confidence intervals. Setup and final release are amortized; this is not a one-shot monitor or disk-throughput benchmark.

## Environment and source

Measured on Aiko on 22 October 2026 (New Zealand time): AMD Ryzen 9 9950X3D, Linux `7.2.6-arch2-1`, Rust `1.99.0`, CPU governor `powersave`. The [environment snapshot](results/ring-state-environment.txt) records CPU topology, compiler details, and executable/lockfile SHA-256 hashes. Both executables use Cargo's default optimized release profile (level 3), without LTO or target-CPU overrides.

Current source: `a45d141c56f3e5f861d340f73e16c8e1e66deba8`. Current executable SHA-256: `ecb095adc99c26f0fa030478391ec6c320acbf314f3eb2f83991364a90cc6922`. The original executable is from `5af3174711b922bb1de78906f9a7e2bc890102e2`, SHA-256 `38664a84bd64754279dc79922abec40ea64a7ff6545982bf777932b39155a781`. Later report and readme edits do not change the measured implementation or build configuration. Actual fixed-file support was checked on this host: the ring-alternation test asserts successful fixed slots on both rings.

The [raw data](results/ring-state.jsonl) contains 280 process runs: 56 local registered/unregistered/original triplets (168 samples) and 56 external registered/unregistered pairs (112 samples), with seven repetitions per configuration. Reproduce with `registration.py` and the original executable as described in the [instructions](readme.md#persistent-ring-state-experiments).

## Findings

- Fixed files improve repeated local 4 KiB reads by **3.7%** with one task/worker and **1.7%** with 16 tasks/four workers against the same implementation with registration disabled. Every paired sample improves in those cases.
- Net change against the original monitor is **+0.9%** for one task/worker (range crosses zero) and **+3.0%** for 16 tasks/four workers. The fixed-file gain does not translate directly into the same net gain: the new lease/cache structure adds work. In particular, disabled-registration 4 KiB reads are **2.9% slower** than the original with one worker.
- Writes and 64 KiB transfers do not establish a general improvement. Some median gains are larger, but their ranges cross zero or are wide; concurrent overlapping writes include substantial filesystem and scheduling variation.
- Alternating rings does not establish a throughput win from fixed files. The 4 KiB external two-driver write case has a **−9.2%** median and a **−15.6% to +25.6%** range. These results are too variable to present cache migration as a speed improvement. The deterministic benefit is avoiding repeated allocation and registration once both entries exist, verified separately by tests.
- Construction, per-ring setup, asynchronous release, and memory proportional to visited rings are real costs. Keep this optional and experimental. Long-running, warm tmpfs transfers do not establish that first-use registration pays for short-lived monitors or storage devices.

The useful result is a shared ownership model and a small, measured local read optimization. Registered buffers and faster completion dispatch remain separate experiments requiring evidence and their own ownership design.

## Local comparison

Throughput medians are thousands of operations per second. Changes are medians of matched-pair ratios, with observed ranges; they need not equal ratios of the throughput medians. `Unregistered/original` exposes added cache/lease overhead without fixed-file benefit.

| Operation | Bytes | Tasks/workers | Original kops/s | Unregistered kops/s | Registered kops/s | Registered/unregistered | Registered/original | Unregistered/original |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| read-at | 4096 | 1/1 | 659.9 | 641.5 | 665.4 | +3.7% (+2.3% to +6.7%) | +0.9% (-0.7% to +4.3%) | -2.9% (-3.2% to -2.3%) |
| read-at | 4096 | 16/4 | 4119.6 | 4193.2 | 4249.3 | +1.7% (+0.5% to +4.6%) | +3.0% (+2.7% to +4.3%) | +1.7% (-0.3% to +2.4%) |
| read-at | 65536 | 1/1 | 419.5 | 419.6 | 436.7 | +4.1% (-0.5% to +6.0%) | +4.4% (-1.3% to +6.7%) | -0.2% (-1.0% to +1.0%) |
| read-at | 65536 | 16/4 | 2225.0 | 2224.5 | 2222.4 | -0.0% (-1.3% to +1.6%) | -0.1% (-1.6% to +1.9%) | -0.2% (-0.4% to +0.6%) |
| write-at | 4096 | 1/1 | 633.1 | 625.1 | 630.7 | +1.0% (-0.5% to +2.6%) | -0.5% (-1.3% to +0.7%) | -1.2% (-3.7% to -0.2%) |
| write-at | 4096 | 16/4 | 3827.7 | 3994.7 | 4023.0 | -2.0% (-3.8% to +32.7%) | +2.9% (+0.4% to +34.6%) | +4.4% (+1.4% to +6.1%) |
| write-at | 65536 | 1/1 | 357.5 | 356.0 | 355.6 | -0.4% (-1.1% to +2.2%) | -0.3% (-1.2% to +0.3%) | -0.6% (-1.8% to +0.7%) |
| write-at | 65536 | 16/4 | 647.8 | 650.9 | 660.0 | +1.7% (-1.3% to +6.6%) | +0.6% (-1.4% to +8.5%) | -0.1% (-6.2% to +6.0%) |

## Deliberate external ring changes

One caller submits through the remote queue, using one driver for the same-ring control and two drivers for alternating rings. Absolute throughput includes remote scheduling and wakeups. This does not measure task stealing or isolate a pure migration instruction cost.

| Operation | Bytes | Drivers | Unregistered kops/s | Registered kops/s | Registered/unregistered |
|---|---:|---:|---:|---:|---:|
| read-at | 4096 | 1 | 271.9 | 275.7 | +0.9% (-2.2% to +2.3%) |
| read-at | 4096 | 2 | 224.3 | 229.6 | +0.0% (-11.6% to +22.0%) |
| read-at | 65536 | 1 | 219.7 | 218.9 | -0.4% (-1.6% to +0.2%) |
| read-at | 65536 | 2 | 189.6 | 193.1 | -0.6% (-11.3% to +12.3%) |
| write-at | 4096 | 1 | 267.9 | 267.4 | -0.4% (-1.1% to +1.5%) |
| write-at | 4096 | 2 | 226.7 | 212.2 | -9.2% (-15.6% to +25.6%) |
| write-at | 65536 | 1 | 208.5 | 208.5 | -0.0% (-0.6% to +0.8%) |
| write-at | 65536 | 2 | 176.7 | 179.3 | +1.0% (-4.3% to +3.0%) |
