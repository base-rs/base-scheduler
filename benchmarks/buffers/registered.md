# Fixed registered buffers

This experiment compares ordinary io_uring `Read`/`Write` with `ReadFixed`/`WriteFixed`, using the same reusable pool and retained-chunk ownership. At 64 KiB, fixed registration improves direct writes by 33–36% with one operation in flight, but provides essentially no write throughput gain at depth 16. It slows the measured cached-file and large TCP cases. Direct I/O deserves a separate policy from buffered I/O.

## What is measured

The [Linux harness](src/bin/registered/linux.rs) drives one ring on one thread. It deliberately bypasses scheduler/executor adapters to isolate the kernel buffer strategy. Both modes use the same executable, pool capacity, aligned payload addresses, slot lookup, queue depth, completion handling, and data validation. File descriptors are ordinary descriptors in both modes; fixed-file registration is excluded.

A pool contains `2 * depth + 1` stable allocations. Each allocation reserves payload size plus alignment slack, and the payload begins on a 4 KiB boundary. The fixed mode registers those payload subranges once. The ordinary mode submits the same pointer and length without registering it. Every submitted operation retains its exclusive lease until its CQE; partial operations retain and resubmit the remaining range. A preallocated completion vector is reused. There is no payload allocation in the timed loop.

`pooled` releases each completed lease immediately. `retained` freezes the completed lease and holds its immutable view through the next completion on that slot, forcing the next operation to use another pool slot. A completed buffer therefore remains available to application parsing independently of subsequent I/O. These are ownership-path measurements, not an HTTP parser or TLS benchmark.

TCP and UDP perform a write followed by an echo read. The JSON `direction` is `read` for these network samples, but both directions are timed. Depth 16 means 16 independent connections with one read or write in flight per connection, not 16 pipelined messages on a single socket. A standard-library peer thread serves each connection. All peers and the ring thread share the sample's CPU affinity. Byte throughput counts one payload direction; echo transports twice that amount. TCP reconstructs full payloads across short completions; UDP checks datagram lengths. Connected-socket `ReadFixed`/`WriteFixed` do not replace the production `recvmsg` API with its source-address, flags, and ancillary-data semantics.

File tests repeatedly access an initialized 8 MiB temporary file in `/home/samuel/.cache/core-scheduler-registered-data`, on ext4 backed by Aiko's Samsung SSD 990 PRO NVMe device `/dev/nvme0n1p2`. Each operation has an explicit position, wrapping within that file. `cached` uses hot page-cache data; `direct` enables `O_DIRECT` with aligned buffers, offsets, and lengths. Direct results are not cold-device benchmarks. Setup populates and synchronizes the file outside timing; writes do not include a timed durability sync. After timing, write verification checks changed data and any untouched suffix.

## Method

Harness and runner source: `5ca168c44f02b30dc9a94cdbf4c28605af469288`. Both measurement runs use executable SHA-256 `2b374a277e76923a0b245f1b6e1086c7556abf2df43b8c80c19f78f421b08189`. Test-only additions made after building do not change the timed paths.

The final release rebuild, including those test-source additions, has SHA-256 `795ce2fc94139acae093239453e14aa26a0f4aa113fd752824e2ee3f09f5d9f4`. The samples above identify the earlier measured executable explicitly.

All builds use Cargo's optimized release profile without `RUSTFLAGS` overrides. The [environment](results/registered-environment.txt) records the compiler, Linux version, filesystem, storage device, and locked-memory limit. Aiko has a Ryzen 9 9950X3D; all samples use CPU affinity 2–5. No builds, tests, profiling, or other benchmark runs were started alongside timing.

The [runner](registered.py) randomizes configuration order and ordinary/fixed order within every matched repetition, with seed 62147. Each sample warms 32 operations per slot, then measures a second pass. Setup registration and final unregistration are measured separately and excluded from throughput timing. Registration setup includes constructing the temporary iovec array, not just syscall latency. Every row records the executable's SHA-256 hash.

The [initial matrix](results/registered.jsonl) contains 840 samples: seven repetitions of both modes, both ownership choices, and depths one and 16. File sizes are 4, 64, and 128 KiB, each measuring 256 MiB. Network payloads are TCP 64 bytes, TCP 64 KiB, and UDP 1,200 bytes, with `max(2000, 20000 / depth)` timed echoes per connection. See the [complete initial summary](results/registered-summary.md).

Some initial cached-file passes last only a few milliseconds, and direct-file samples vary substantially. A longer validation repeats representative gains, losses, and network cases with 11 matched repetitions, 4 GiB per file pass, and `max(2000, 100000 / depth)` network echoes per connection. It uses the identical executable and ownership paths. Results are medians of per-repeat fixed/ordinary throughput ratios, not ratios of independent throughput medians. Pair ranges show observed variation, not confidence intervals. All results are from one host.

## Results

The [352 longer-validation samples](results/registered-validation.jsonl) give these matched median throughput ratios. A ratio above one favors fixed registration. All file and TCP rows below use 64 KiB payloads; UDP uses 1,200 bytes.

| Workload | Depth | Immediate lease return | Retained frozen view |
| --- | ---: | ---: | ---: |
| Cached read | 16 | 0.918× | 0.925× |
| Cached write | 16 | 0.936× | 0.917× |
| Direct read | 1 | 1.051× | 1.042× |
| Direct read | 16 | 0.983× | 0.994× |
| Direct write | 1 | 1.356× | 1.331× |
| Direct write | 16 | 1.000× | 1.000× |
| TCP echo | 16 connections | 0.957× | 0.950× |
| UDP echo | 1 connection | 1.008× | 0.979× |

The direct-write depth-one improvement appears in all 22 longer matched pairs, including both ownership choices. Independent throughput medians are 2.118 → 2.885 GiB/s for immediate return and 2.135 → 2.811 GiB/s with retained views. Ratios of those medians differ from the matched ratios above. The initial seven-repeat run also favors fixed registration in both corresponding configurations. This is a repeatable result on this host, not a promise of the same improvement on other devices or working sets.

Direct reads at depth one are too variable for a firm performance claim: individual fixed/ordinary pairs range from 0.480× to 2.294× across the ownership variants. More elapsed time did not eliminate that variability. Depth-16 direct writes show essentially equal throughput, and direct reads at depth 16 show no gain. UDP also fails to establish a consistent advantage. The large TCP and cached-read regressions persist across both ownership choices and every longer matched pair. Cached writes favor ordinary buffers in 21 of 22 longer pairs.

See the [complete validation summary](results/registered-validation-summary.md) for every throughput median, pair range, and setup cost. Median fixed-registration setup is 3.18 µs for three 64 KiB slots and 18.80 µs for 33 slots; corresponding unregistration costs are 1.44 and 5.55 µs. These costs are outside the throughput timer, so amortizing initial registration does not explain the measured slowdowns. The harness measures those slowdowns; it does not profile their kernel instruction-level cause.

## Resource limits

Aiko's locked-memory limit is 8 MiB. Early trials failed registration with `ENOMEM` below the apparent payload budget. Both comparison modes now refault their exclusively owned whole payload pages with transparent huge pages disabled, and fixed samples explicitly unregister completed pools before destroying their rings. The complete runs succeeded with both controls; the experiment does not isolate their individual contributions to the earlier failure. Small payloads do not discard allocator pages.

The kernel [accounts registered pages against per-user locked memory](https://github.com/torvalds/linux/blob/v7.2/io_uring/rsrc.c), and [ring destruction queues asynchronous cleanup](https://github.com/torvalds/linux/blob/v7.2/io_uring/io_uring.c), which can overlap rapidly launched processes. A production pool must handle registration capacity and resource retirement explicitly. The largest measured registration is 33 × 128 KiB, or 4.125 MiB of payload storage, excluding allocator slack and bookkeeping.

## Interpretation

Fixed registration and provided buffers are different mechanisms. [Fixed registration](https://man7.org/linux/man-pages/man3/io_uring_register_buffers.3.html) amortizes page mapping and reference management for direct I/O; it does not itself eliminate socket payload copies. This experiment does not measure provided-buffer rings, receive multishot, `SEND_ZC`, NIC zero copy, or integrated Native scheduler throughput. The earlier [shared-pool contention improvements](fast-path.md) measure userspace pool synchronization and must not be attributed to kernel registration.

Keep the portable lease and frozen-view model. It permits retaining completed chunks while other slots serve new I/O without requiring protocol code to understand registration. The backend can use stable allocations for fixed registration where measured workloads benefit. Keep syscall-first persistent-monitor receives available, and do not force ordinary sockets or cached files through a fixed-buffer path on the assumption that registration must be faster. A shared provided-buffer receive strategy integrated with these leases remains separate work.

## Reproduce

```sh
cargo build --release --locked -p core-scheduler-buffer-benchmark --bin registered
python3 benchmarks/buffers/registered.py \
    --binary target/release/registered \
    --directory /path/on/ext4 \
    --output benchmarks/buffers/results/registered.jsonl
python3 benchmarks/buffers/registered.py \
    --binary target/release/registered \
    --directory /path/on/ext4 \
    --validation --repeats 11 --file-mib 4096 --network-messages 100000 \
    --output benchmarks/buffers/results/registered-validation.jsonl
python3 benchmarks/buffers/summarize_registered.py benchmarks/buffers/results/registered.jsonl
python3 benchmarks/buffers/summarize_registered.py benchmarks/buffers/results/registered-validation.jsonl
```

The harness assumes 4 KiB base pages, as on Aiko. Use a Linux host that permits io_uring, supports the selected filesystem's `O_DIRECT` behavior, and has sufficient locked-memory allowance. Run the two measurement commands sequentially, without competing builds or profiling.

## Verification

Linux tests exercise ordinary and fixed operations across TCP, UDP, cached files, and direct files, with immediate returns and retained views. Additional cases cover short TCP completions, small allocations that must not discard allocator pages, a genuinely truncated input file, command-line JSON output, and abandoning a submitted operation without releasing its backing storage. Linux workspace coverage passes at 28,314/28,314 measured source regions; macOS passes at 21,784/21,784, including the unsupported-platform diagnostic. Linux and macOS Clippy checks pass, and the benchmark package checks with Rust 1.85 on macOS. Linux's platform-specific code is compiled and tested with the compiler recorded in the environment file. Independent review approved the harness. No production scheduler implementation or public API changes are made by this experiment.
