# File monitor benchmark

Compare ordinary owned file operations with reusable `FileMonitor` operations on Direct, the Tokio adapter, Native readiness, and Native io_uring. This benchmark isolates repeated positioned reads and writes; it does not measure disk throughput or durability.

## Run

Build every variant in the same optimized executable:

```sh
cargo build --release --locked -p core-scheduler-file-benchmark
TMPDIR=/dev/shm flock /tmp/core-scheduler-perf.lock python3 benchmarks/files/run.py --affinity --output /tmp/files.jsonl
python3 benchmarks/files/summarize.py /tmp/files.jsonl
```

`--affinity` is Linux-specific and uses CPU 2 for one worker and CPUs 0–7 for four workers. Adjust these CPU sets for other machines. `--backends iouring` narrows the run; `--repeats` and `--iterations` control its duration. Avoid other builds, tests, or benchmarks during measurement.

The executable also accepts one case directly:

```sh
target/release/core-scheduler-file-benchmark iouring monitor read-at 16 50000 4 4096
```

Arguments are backend, mode (`ordinary` or `monitor`), workload (`read-at` or `write-at`), tasks, iterations per task, workers, and bytes per operation. `native` selects readiness; `iouring` selects Native's executor with per-worker io_uring drivers. `tokio` measures the Core Tokio adapter, not raw `tokio::fs`.

## Method

Each process seeds a temporary file before timing, then repeatedly transfers an owned `Vec` at offset zero. One task or 16 tasks share that file; each task reuses its own buffer and monitor. Reads use a page-cache-hot file. Concurrent writes overlap the same range, deliberately including filesystem contention. Setting `TMPDIR=/dev/shm` selects tmpfs on the measurement host. No synchronization to durable storage is included.

The runner randomizes matched ordinary/monitor pairs with a fixed seed. The default matrix has seven pairs per case, 50,000 timed iterations per task, 4 KiB and 64 KiB transfers, and one task/one worker or 16 tasks/four workers. Every task executes one additional first operation to populate its request cache. Both modes construct and close a monitor, so they have comparable setup boundaries.

`operations_per_second` divides tasks × iterations by wall time from task spawning through joining. This wall time includes task and buffer setup, each task's extra first operation, and monitor close; it excludes file setup and file close. `mean_operation_ns` instead divides the sum of per-task elapsed times after their first operation through monitor close by tasks × iterations. This second metric is concurrent per-task latency, not the inverse of aggregate throughput. The report uses throughput only; at 50,000 iterations, setup is amortized.

Tables show independently computed throughput medians and the median percentage change of matched pairs. The accompanying range is the minimum and maximum paired change, not a confidence interval. Small changes and wide ranges warrant caution. These results do not establish performance for sequential-cursor operations, cold files, physical disks, or application parsers.

## Implementation tradeoff

The io_uring monitor keeps a typed request allocation and completion slot between operations. Reusing the same buffer-owner type avoids new request and completion-channel allocations after warmup. Changing owner type replaces the cached request. The allocation test measures submission and completion on the owning worker: 32 warmed monitor operations allocate zero times, while 32 ordinary operations allocate 64 times.

Direct, Tokio, and readiness preserve their existing operation implementations and add a reusable retirement guard. They do not gain the io_uring request cache. Their expected benefit is explicit sequential state and retirement, with a small tracking cost; the monitor is not an unconditional throughput optimization.

The monitor retains a file and admits one operation at a time. Multiple monitors may share a file, but share its OS cursor too; explicit offsets provide independent positions. Neither fixed-file registration nor registered buffers are part of this prototype. See the [interface design](../../context/design.md) for lifecycle and compatibility rules.

## Measurements

See [results](results.md) for the measured comparison, source identity, and raw samples.
