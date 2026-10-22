# Shared payloads and worker-local provided-buffer rings

This is a historical shared-budget prototype. The [worker-owned pool experiment](worker-owned.md) replaces cross-worker reclamation with eagerly provisioned local storage.

One logical pool supplies exactly its requested payload capacity across connections and workers. The io_uring backend now implements that contract with a provided-buffer ring per participating worker. Payload storage is shared; ring metadata is local. This is a working prototype, not evidence that provided buffers universally outperform syscall-first receives.

## Where ownership lives

There is no global buffer-to-worker map. A free slot belongs to the heap pool's atomic availability bitmap. Publication transfers its unique lease into a worker-local table indexed by the buffer ID submitted to the kernel. A receive CQE removes that entry and hands the completed writable lease to the application. The last mutable or frozen owner returns the slot directly to the shared pool, regardless of the thread that releases it. Completed payloads have no ring affinity.

Each worker publishes at most eight buffers at once. This bounds its initial reservation and metadata without multiplying payload capacity by worker count. Receive metadata occupies an additional aligned prefix in each allocation; the public buffer still exposes exactly the requested usable size and alignment. Free buffers can move between workers without copying payload bytes or deregistering fixed memory: provided-buffer rings are distinct from fixed-buffer registration.

Still-published storage cannot move immediately. Under pressure, the pool asks donor workers to suspend and cancel their multishot receives, drain terminal original completions, unregister the provided-buffer ring, and release unconsumed leases. Application-held chunks remain untouched. Empty groups initially poll socket readiness before claiming capacity, preventing idle receivers from repeatedly recalling busy workers' storage. Capacity waiting reuses one boxed waiter per worker instead of allocating a future for every exhausted batch.

The receive remains on the ring chosen at construction. A consumer task may migrate independently. Cross-worker completion wakeups and heap-pool returns remain possible; this prototype does not pin tasks or migrate active multishot requests.

## Aiko measurements

The [main dataset](results/worker-pools.jsonl) contains 168 optimized samples: seven randomized matched pairs for each of twelve cases. The baseline is commit `fd41dd6`, using the existing shared pool and syscall-first readable monitor; the candidate is commit `2176979`, the shared provided-ring implementation with eight ring entries. The existing comparison runner labels these binaries `concrete` and `contract` respectively; these labels identify the before/after executable, not a difference in the public pool contract. Every row records its executable SHA-256.

Aiko ran Linux 7.2.6-arch2-1, Rust 1.99.0, and a Ryzen 9 9950X3D. Both binaries used Cargo's optimized release profile and CPU affinity 2–9. Builds and tests did not overlap timing. Each connection warmed 1,000 exchanges and measured 10,000; one request byte triggered a burst of eight response payloads. All connections shared one pool with eight buffers per connection. `pool` releases chunks after use; `shared` freezes and retains a parser view until a later receive. These are loopback transport exercises, without protocol parsing, TLS, or physical-network traffic.

Median changes in messages per second, computed from per-repeat matched ratios:

| Transport and payload | 1 worker / 1 connection, pool | 1 worker / 1 connection, retained | 2 workers / 8 connections, pool | 2 workers / 8 connections, retained |
| --- | ---: | ---: | ---: | ---: |
| TCP, 64 bytes | +1.6% | +2.3% | −20.6% | −25.3% |
| TCP, 64 KiB | +2.2% | +2.4% | −16.1% | −23.3% |
| UDP, 1,200 bytes | −0.3% | −0.7% | −7.3% | −11.5% |

The geometric mean across the twelve cell medians is 0.913× the baseline. Single-connection results are close, with modest gains in several TCP cases, but the multi-connection regressions preclude recommending this as a universal faster default. Multi-connection paired ratios also vary widely, sometimes changing sign; they do not establish stable improvement in any of those cases. Eight peer threads plus two runtime workers share eight allowed CPUs, so host scheduling is part of this workload.

A [follow-up dataset](results/worker-pools-32.jsonl) tests a 32-entry publication window in the six multi-connection cases, with five matched repeats each. The TCP medians change inconsistently, from −21.1% to +59.1% for small messages and −10.5% to +18.8% for large ones; retained UDP is −52.7%. This does not support retaining a larger fixed window. The implementation keeps eight entries. Neither run isolates ring placement, consumer migration, result-queue synchronization, refill overhead, or syscall count sufficiently to attribute the regression to one cause.

The initial measurement attempt exposed a real shutdown race: close sometimes observed the multishot request's terminal `ENOBUFS` CQE and reported exhaustion as a close failure. Retirement now treats that CQE as successful shutdown, before cleanup; registration and deregistration errors remain observable. The main dataset was collected after the final change separating completion retirement from setup failures, and finished successfully. The 32-entry experiment predates that final error-routing change. An [earlier eight-entry run](results/worker-pools-initial.jsonl), also with 168 samples, showed the same broad multi-connection regression with different cell medians (0.886× geometric mean). It is retained to show the sensitivity across runs; it is not pooled with the final samples.

Subsequent independent review found that a capacity waiter could lose a returned slot to another worker's publication and then stall without requesting reclamation again. Worker and public-pool acquisition now repeat reclamation after exhausted retries, with deterministic regression tests. This pressure-path correction has not been rebenchmarked; the results above describe the measured prototype, not the corrected revision.

## Validation and next steps

The measured prototype passed Linux and macOS workspace tests, documentation tests, Clippy, and the measured source-region coverage gates: Linux covered 31,019/31,019 regions and macOS covered 22,009/22,009. Tests exercise four workers sharing two buffers, retained payload address stability, cross-thread release, alignment, UDP metadata and truncation, cancellation, mixed ordinary and persistent receives, reclamation, driver loss, and deregistration failure. The review correction adds worker and public-acquisition regressions for a returned slot being republished before its waiter runs. An exceptional failure retains storage whose kernel retirement cannot be established rather than returning it for unsafe reuse.

Keep the ownership model as the basis for further experiments. Before promoting provided-buffer receives as the performance default, profile the multi-connection cases and isolate initial ring placement, cross-worker wakeups, result delivery, and exhaustion. A larger publication table alone has not solved the observed gap. No fixed-buffer file I/O or zero-copy send integration is included here.

## Reproduce

Build the baseline and candidate in separate checkouts with `cargo build --release --locked -p core-scheduler-buffer-benchmark --bins`, then run on an otherwise idle Linux host:

```sh
flock -x /tmp/core-scheduler-perf.lock \
  python3 benchmarks/buffers/pool_contract.py \
  --before /path/to/baseline/release --after /path/to/candidate/release \
  --repeats 7 --socket-iterations 10000 --match '^sockets iouring' \
  --output /tmp/worker-pools.jsonl
python3 benchmarks/buffers/summarize_pool_contract.py \
  /tmp/worker-pools.jsonl --output /tmp/worker-pools-summary.json
```

The runner expects the `leases`, `sockets`, and `files` executables in each directory. For the 32-entry experiment, change only the entry limit in `provided_buffers.rs`, rebuild, and restrict the match to `'^sockets iouring.* 2 (tcp|udp)'` with five repeats. Do not run builds, coverage, or other benchmarks concurrently.
