# File monitor performance

This records the original request-reuse prototype. The later [ring-state experiment](ring-state-research.md) adds fixed-file registrations and compares against this executable separately.

The prototype removes per-operation io_uring request and completion-channel allocations and modestly improves small warm reads. It is not a general throughput improvement: larger transfers barely change, writes are mixed, and fallback backends usually pay a small retirement-tracking cost. Keep this capability optional and experimental while its state can also support future backend-specific registration.

## Environment and source

Measured on Aiko on 22 October 2026 (New Zealand time), using an AMD Ryzen 9 9950X3D, Linux `7.2.6-arch2-1`, and Rust `1.99.0`. The CPU governor was `powersave`. One-worker processes were pinned to CPU 2; four-worker processes were pinned to CPUs 0–7, within one CCD. The [environment snapshot](results/environment.txt) records compiler details and SHA-256 hashes of the executable and Cargo lockfile.

Source commit: `5af3174711b922bb1de78906f9a7e2bc890102e2`, based on main `6b9f8a3b12ce94a98a938eea9afce1a3d3ede39d`. Ordinary and monitor paths ran in the same release executable (`cargo build --release --locked -p core-scheduler-file-benchmark`), with Cargo's default release optimization level 3, no LTO override, and no target-CPU override. Later changes to documentation, results, and manifest whitespace do not change the measured code or build configuration.

The [raw samples](results/aiko.jsonl) contain 448 process runs: seven randomized matched pairs for each backend, workload, transfer size, and task/worker configuration. Each task performs 50,000 measured iterations, reuses its owned `Vec`, and operates at offset zero in the same tmpfs file. Concurrent writes overlap and include filesystem contention. No physical-disk or durability measurements are included. See the [benchmark method](readme.md#method) for timing boundaries and reproduction commands.

## Findings

- io_uring 4 KiB reads improve by a median paired **5.7%** with one task/worker and **2.2%** with 16 tasks/four workers. Every matched pair improved in those two cases.
- io_uring 64 KiB reads change by **+1.2%** and **+0.5%**. Payload and filesystem costs increasingly dominate bookkeeping.
- io_uring writes are mixed: one-task 4 KiB writes lose **0.8%**; 64 KiB writes are effectively unchanged. The apparent **+2.7%** improvement for concurrent 4 KiB writes has a **−11.1% to +32.8%** paired range, so it is not a reliable gain.
- Direct and Native readiness generally lose **0–3%**, reaching **3.2%** for Direct one-task 4 KiB writes. They retain their ordinary dispatch path and add retirement tracking.
- The Tokio adapter ranges from **−2.0% to +2.7%** across median paired changes. Its positive small-read result has a range crossing zero; the monitor has no Tokio-specific optimization to explain it. Treat small gains there as experimental variation, not an established optimization.
- The owning-worker allocation test counts **zero allocations across 32 warmed io_uring monitor operations**, versus **64 allocations across 32 ordinary operations**. This applies when the buffer-owner type is reused and backend capacity already exists; construction and changing owner type still allocate.

The clearer reason to retain a monitor is reusable sequential operation state with explicit retirement. These measurements support a small read optimization in io_uring, not requiring monitors for every file operation. Fixed-file and fixed-buffer registration are not implemented or measured.

## Full comparison

Throughput is in thousands of operations per second. Changes are medians of matched-pair ratios; they need not equal the ratio of the two displayed throughput medians. Ranges are observed pair minima and maxima, not confidence intervals. `native` means Native readiness, `iouring` means Native with per-worker io_uring, and `tokio` means the Core Tokio adapter.

| Backend | Operation | Bytes | Tasks/workers | Ordinary kops/s | Monitor kops/s | Paired change (range) |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| direct | read-at | 4096 | 1/1 | 602.0 | 589.4 | -2.2% (-4.6–-1.3%) |
| direct | read-at | 4096 | 16/4 | 2779.7 | 2790.6 | -0.8% (-1.7–+0.8%) |
| direct | read-at | 65536 | 1/1 | 398.8 | 393.0 | -1.2% (-2.8–-0.9%) |
| direct | read-at | 65536 | 16/4 | 2055.6 | 2042.9 | -0.4% (-1.4–+1.6%) |
| direct | write-at | 4096 | 1/1 | 600.0 | 578.7 | -3.2% (-4.1–-2.8%) |
| direct | write-at | 4096 | 16/4 | 2794.0 | 2787.1 | +0.3% (-1.5–+1.5%) |
| direct | write-at | 65536 | 1/1 | 342.1 | 336.9 | -1.4% (-2.4–-0.8%) |
| direct | write-at | 65536 | 16/4 | 668.5 | 669.7 | -0.4% (-1.5–+1.5%) |
| iouring | read-at | 4096 | 1/1 | 614.3 | 650.1 | +5.7% (+2.7–+6.2%) |
| iouring | read-at | 4096 | 16/4 | 4076.8 | 4161.9 | +2.2% (+1.0–+3.5%) |
| iouring | read-at | 65536 | 1/1 | 414.7 | 419.5 | +1.2% (+0.2–+1.5%) |
| iouring | read-at | 65536 | 16/4 | 2217.3 | 2224.9 | +0.5% (-1.0–+2.8%) |
| iouring | write-at | 4096 | 1/1 | 633.1 | 624.4 | -0.8% (-3.2–-0.2%) |
| iouring | write-at | 4096 | 16/4 | 3808.0 | 3892.8 | +2.7% (-11.1–+32.8%) |
| iouring | write-at | 65536 | 1/1 | 354.1 | 354.9 | +0.6% (-1.2–+2.5%) |
| iouring | write-at | 65536 | 16/4 | 656.5 | 655.3 | -0.2% (-5.2–+1.6%) |
| native | read-at | 4096 | 1/1 | 586.4 | 572.5 | -2.2% (-3.9–-1.6%) |
| native | read-at | 4096 | 16/4 | 2153.0 | 2120.1 | -1.4% (-2.5–+0.1%) |
| native | read-at | 65536 | 1/1 | 345.0 | 336.8 | -2.4% (-2.9–-2.0%) |
| native | read-at | 65536 | 16/4 | 1863.0 | 1862.6 | +0.0% (-0.3–+1.1%) |
| native | write-at | 4096 | 1/1 | 564.2 | 557.4 | -1.0% (-2.7–-0.2%) |
| native | write-at | 4096 | 16/4 | 2171.3 | 2138.7 | -1.3% (-2.9–-0.0%) |
| native | write-at | 65536 | 1/1 | 298.7 | 297.1 | -0.8% (-1.8–+0.3%) |
| native | write-at | 65536 | 16/4 | 668.5 | 666.2 | -0.7% (-2.8–+2.3%) |
| tokio | read-at | 4096 | 1/1 | 533.5 | 548.8 | +2.7% (-2.2–+4.4%) |
| tokio | read-at | 4096 | 16/4 | 2191.3 | 2217.8 | +0.8% (-0.4–+1.5%) |
| tokio | read-at | 65536 | 1/1 | 337.0 | 331.5 | -2.0% (-2.3–-1.2%) |
| tokio | read-at | 65536 | 16/4 | 2032.0 | 2009.6 | -1.0% (-1.9–-0.2%) |
| tokio | write-at | 4096 | 1/1 | 527.9 | 526.5 | -0.5% (-3.7–+1.9%) |
| tokio | write-at | 4096 | 16/4 | 2225.1 | 2264.1 | +1.4% (+1.0–+6.0%) |
| tokio | write-at | 65536 | 1/1 | 286.8 | 284.8 | -1.0% (-1.8–-0.0%) |
| tokio | write-at | 65536 | 16/4 | 674.2 | 672.2 | -0.4% (-2.8–+2.4%) |
