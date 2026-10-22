# File registration migration cost

Keeping only the current ring's passive file registration looks reasonable when migration is infrequent. It avoids retaining monitor state on every visited worker. The cost is measurable under deliberate migration on every operation, and falls substantially when several operations run before the next migration. The implementation now adopts the invalidation policy. This report preserves the original measured baseline and candidate patch for reproduction at their recorded source commit.

## Policies and lifetime

The retained policy caches one registration per visited ring. Its current entry is inline; revisiting another worker reuses that worker's lease. For N monitors and M visited workers, this can retain N × M monitor entries. The default table capacity of 256 bounds successful fixed-file registrations per ring, not monitor cache entries: failed or full-table registrations retain a fallback lease. The limit is configurable and does not cap the number of usable ordinary file descriptors.

The invalidation candidate retains one current entry. A new operation on another ring queues release on the original controller and creates a new lease on the destination. It also retains pending and failed evictions until close can observe them; successful releases are reaped after acknowledgment. Thus it eliminates the permanent visited-worker cache, but is not a strict bound of one allocation or one kernel registration at every instant. A stalled original driver can delay release. In-flight operations remain on their submitting ring; moving the monitor alone does not migrate accepted work.

Request and completion storage remain reusable across migration in both policies. Invalidation adds registration/retirement allocation on each switch, a retiring-list check during reuse, and fixed-file update syscalls. This is not simply changing a ring identity. Failed release remains observable across later migration and repeated explicit close. Unit tests cover those guarantees.

## Findings

- Without migration, the sixteen case medians range from −0.5% to +1.3%; most observed pair ranges cross zero. The candidate does not show a consistent material same-worker penalty in this harness, although its bookkeeping and generated code differ.
- With two workers and a switch after every operation, median changes range from −5.2% to +0.2%. After 16 operations, they range from −2.3% to +0.7%. Changes at 256 operations are within ±0.7% by case median; at 4096 they range from −0.1% to +1.3%.
- Across 48 rings, switching every operation costs about 9–10% by case median. The median added batch time is 0.79–0.92 microseconds. After 16 operations, median throughput costs are 1.0–3.3%. These timing differences include release and scheduling effects; they are not isolated registration syscall durations.
- At 4096 operations per migration, the four 48-ring case medians range from −0.5% to +1.0%. The 256-operation results are noisier, including a −3.6% write median with a −13.1% to +3.0% paired range. Do not claim a universal zero-cost threshold from these samples.
- Independent syscall counting confirms two additional file-table update calls per timed migration. With 101 measured single-operation batches (100 migrations), retained/invalidation totals are 8/208 calls for two rings and 192/392 for 48 rings. Both single-ring controls make four calls. Counts include setup, warmup, and close. Tracing was separate from throughput measurement; traced durations are not used.

My recommendation is to prefer invalidation for this passive file cache rather than retain every visited worker, given the intended policy of infrequent stealing and localized work. The data does not establish the actual migration frequency in an application, nor does it automatically apply to active socket polls, timers, or provided-buffer pools. Frequent deliberate migration still benefits from retention. A small bounded cache is a possible later compromise if a real workload demonstrates repeated migration is important; no such cache is implemented here.

## Method and limits

One coordinator transfers the same monitor and owned buffer to a specific entered ring worker. That worker executes a batch of positioned operations and returns the monitor and buffer. Batches rotate deterministically across workers. This exercises actual monitor ownership migration and local TLS routing, without relying on executor work stealing. Idle drivers continue processing releases. The first measured batch remains on the last warmup worker, so a multiworker run has rounds − 1 timed migrations; a single-worker run has none.

Warmup visits every ring and initializes tables, request storage, and driver queues. The retained policy keeps all warmed leases; invalidation deliberately discards earlier registrations. Files are page-cache-hot tmpfs files at offset zero, with 4 KiB or 64 KiB transfers and no durable synchronization. This measures one sequential stream, not many concurrently busy files or disk throughput.

There are seven randomized matched repetitions of each worker/batch/size/read-write combination, 672 process runs in total. Matched policies execute consecutively in randomized order, with a fixed seed. Round counts are clamped: batch sizes 1, 16, 256, and 4096 use 8192, 4096, 256, and 64 rounds, or 8192, 65536, 65536, and 262144 measured operations respectively. Compare policies within each case; durations and operation totals differ between batch sizes.

The coordinator is pinned to CPU 0. One- and two-worker processes use CPUs 0–7, with their workers pinned to CPUs 1 and 2. The 48-worker process uses CPUs 0–31; its workers share 31 logical CPUs while CPU 0 serves the coordinator. Aiko has 16 physical cores and 32 hardware threads: this is an oversubscribed 48-ring lifecycle experiment, not 48-core throughput scaling. The raw rows include actual worker assignments.

Batch latency includes coordinator channels, eventfd wakes, worker parking, and I/O. Mean batch latency and p50/p99 exclude final close. Throughput includes the measured loop and explicit monitor close, but excludes setup, file close, and selector shutdown. The table reports median paired throughput changes and observed minimum/maximum paired changes, not confidence intervals. A median change need not equal a ratio of independently calculated throughput medians.

## Source and reproduction

Measured on Aiko on 22 October 2026: Ryzen 9 9950X3D, Linux 7.2.6-arch2-1, Rust 1.99.0, powersave governor. Both policies use the default optimized Cargo release profile, without LTO or target-CPU overrides. Runs were serialized; no builds or tests ran concurrently with timing.

Harness and retained source: `bd9aba9729318a9350b764555859d138409ff33b`, based on the file-monitor prototype `803622acaa969ea8019ccf436ea9c459638447ea`. The [candidate patch](experiments/invalidate-on-migration.patch) has SHA-256 `ba1ad067ec75b9dc7c1965ef5aa4988d60d2f307c47f802969921ebee1ef0232`. The [environment snapshot](results/migration-cost-environment.txt) records compiler, lockfile, and executable hashes; [raw results](results/migration-cost.jsonl) preserve every sample. Later wording and report changes do not affect the measured executable.

From a clean worktree at the recorded harness source commit (`bd9aba9729318a9350b764555859d138409ff33b`), build the two binaries. The patch is already incorporated into the current implementation; do not apply it again to the current branch:

```sh
cargo build --release --locked -p core-scheduler-file-benchmark --bin migration
cp target/release/migration /tmp/migration-cost-retain
patch -p1 < benchmarks/files/experiments/invalidate-on-migration.patch
cargo build --release --locked -p core-scheduler-file-benchmark --bin migration
cp target/release/migration /tmp/migration-cost-invalidate
patch -R -p1 < benchmarks/files/experiments/invalidate-on-migration.patch
TMPDIR=/dev/shm flock /tmp/core-scheduler-perf.lock python3 benchmarks/files/migration.py --retain /tmp/migration-cost-retain --invalidate /tmp/migration-cost-invalidate --output /tmp/migration-cost.jsonl
```

The runner's CPU sets are specific to this host. Adjust them before using another machine. The binary also accepts `WORKERS BURST ROUNDS SIZE [read|write] [unregistered]`. Tests exercise read/write, registered/unregistered, one/two-worker handoff, and explicit close. Both policy builds passed the io_uring unit tests and benchmark tests with warnings denied. The retained workspace also passed the full all-feature Linux source-region coverage gate (24985/24985 measured regions).

Count registration calls separately, without using traced timings:

```sh
TMPDIR=/dev/shm taskset -c 0-31 strace -f -qq -e io_uring_register -c /tmp/migration-cost-invalidate 48 1 101 4096 read
```

## Complete paired results

Negative changes mean invalidation is slower. Batch delta is the median difference of matched mean batch latencies; large-batch noise and same-worker differences are not pure migration cost.

| Workers | Operations per batch | Bytes | Operation | Invalidation throughput change (observed range) | Batch delta (µs) |
|---:|---:|---:|---|---:|---:|
| 1 | 1 | 4096 | read-at | +1.0% (-2.5% to +6.1%) | -0.04 |
| 1 | 1 | 4096 | write-at | -0.5% (-3.4% to +3.0%) | +0.02 |
| 1 | 1 | 65536 | read-at | +0.9% (-2.3% to +5.5%) | -0.05 |
| 1 | 1 | 65536 | write-at | +0.2% (-4.0% to +4.0%) | -0.01 |
| 1 | 16 | 4096 | read-at | +0.6% (-0.5% to +1.4%) | -0.17 |
| 1 | 16 | 4096 | write-at | +0.8% (-0.5% to +2.5%) | -0.20 |
| 1 | 16 | 65536 | read-at | +0.4% (-1.6% to +2.5%) | -0.16 |
| 1 | 16 | 65536 | write-at | -0.1% (-0.3% to +1.7%) | +0.07 |
| 1 | 256 | 4096 | read-at | +1.3% (-0.2% to +1.8%) | -4.62 |
| 1 | 256 | 4096 | write-at | +0.6% (-0.6% to +0.9%) | -2.12 |
| 1 | 256 | 65536 | read-at | +0.1% (-0.3% to +0.8%) | -0.85 |
| 1 | 256 | 65536 | write-at | +0.7% (+0.0% to +1.5%) | -4.90 |
| 1 | 4096 | 4096 | read-at | +0.3% (-1.0% to +1.4%) | -19.89 |
| 1 | 4096 | 4096 | write-at | +0.6% (-0.2% to +1.6%) | -36.18 |
| 1 | 4096 | 65536 | read-at | +0.8% (-1.2% to +1.7%) | -74.36 |
| 1 | 4096 | 65536 | write-at | -0.2% (-0.6% to +0.3%) | +19.13 |
| 2 | 1 | 4096 | read-at | +0.2% (-7.5% to +2.3%) | -0.01 |
| 2 | 1 | 4096 | write-at | -1.0% (-5.6% to +3.7%) | +0.04 |
| 2 | 1 | 65536 | read-at | -5.2% (-6.3% to +0.7%) | +0.28 |
| 2 | 1 | 65536 | write-at | -4.0% (-8.6% to +1.0%) | +0.23 |
| 2 | 16 | 4096 | read-at | -2.3% (-3.1% to -1.1%) | +0.59 |
| 2 | 16 | 4096 | write-at | -2.1% (-2.8% to -1.3%) | +0.55 |
| 2 | 16 | 65536 | read-at | +0.7% (-2.6% to +2.2%) | -0.27 |
| 2 | 16 | 65536 | write-at | -0.9% (-1.4% to -0.4%) | +0.43 |
| 2 | 256 | 4096 | read-at | +0.6% (-0.1% to +0.9%) | -2.29 |
| 2 | 256 | 4096 | write-at | -0.2% (-0.9% to +2.0%) | +0.86 |
| 2 | 256 | 65536 | read-at | +0.1% (-1.1% to +1.8%) | -0.82 |
| 2 | 256 | 65536 | write-at | -0.2% (-0.6% to +0.2%) | +1.24 |
| 2 | 4096 | 4096 | read-at | +0.5% (-0.7% to +1.4%) | -32.84 |
| 2 | 4096 | 4096 | write-at | +1.3% (+0.0% to +2.4%) | -79.21 |
| 2 | 4096 | 65536 | read-at | +0.6% (-0.1% to +1.6%) | -54.62 |
| 2 | 4096 | 65536 | write-at | -0.1% (-1.3% to +0.1%) | +13.01 |
| 48 | 1 | 4096 | read-at | -10.2% (-14.0% to -8.6%) | +0.79 |
| 48 | 1 | 4096 | write-at | -10.2% (-14.8% to -8.7%) | +0.84 |
| 48 | 1 | 65536 | read-at | -9.4% (-11.5% to +6.4%) | +0.88 |
| 48 | 1 | 65536 | write-at | -9.2% (-11.1% to -5.9%) | +0.92 |
| 48 | 16 | 4096 | read-at | -3.0% (-5.2% to -1.5%) | +0.94 |
| 48 | 16 | 4096 | write-at | -3.3% (-6.7% to -2.0%) | +1.01 |
| 48 | 16 | 65536 | read-at | -1.7% (-6.4% to +2.5%) | +0.75 |
| 48 | 16 | 65536 | write-at | -1.0% (-2.0% to -0.3%) | +0.54 |
| 48 | 256 | 4096 | read-at | +0.6% (-9.1% to +13.0%) | -3.63 |
| 48 | 256 | 4096 | write-at | -3.6% (-13.1% to +3.0%) | +22.16 |
| 48 | 256 | 65536 | read-at | -0.1% (-11.3% to +2.0%) | +1.59 |
| 48 | 256 | 65536 | write-at | -1.0% (-2.4% to +0.6%) | +9.98 |
| 48 | 4096 | 4096 | read-at | +0.5% (+0.0% to +2.2%) | -31.33 |
| 48 | 4096 | 4096 | write-at | +0.7% (+0.3% to +3.5%) | -43.00 |
| 48 | 4096 | 65536 | read-at | +1.0% (-4.3% to +5.4%) | -87.82 |
| 48 | 4096 | 65536 | write-at | -0.5% (-1.2% to +1.0%) | +54.91 |
