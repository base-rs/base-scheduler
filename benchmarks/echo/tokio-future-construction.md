# Constructing Tokio socket operation futures

The Tokio adapter now passes a future factory to its private `socket_result` helper, constructing the I/O future inside its pinned local after the existing admission checks. This avoids retaining an already constructed nested future as an argument and moving it again when pinning. The public interface, buffer ownership, cancellation ordering, and readiness policy stay unchanged.

In the inspected optimized Linux build, scalar read and write setup each made two `memcpy` calls of 264 bytes. These moved nested future state, not payload bytes. The factory variant eliminates those calls in both operation poll functions. The [before assembly](results/tokio-future-construction/baseline/adapter-assembly.txt) and [after assembly](results/tokio-future-construction/factory/adapter-assembly.txt) preserve the inspected functions. This is a compiler/build observation, not a Rust guarantee that future moves are always elided.

## Aiko comparison

Twenty-four uninstrumented samples compare the baseline and factory builds, including unchanged raw-Tokio controls. Each row shows the median of three runs with one server worker, 32 persistent TCP connections, one second of warmup, and three measured seconds. All cases run sequentially in shuffled order with the same fixed client.

| Backend | Payload | Baseline round trips/s | Factory round trips/s | Change |
| --- | ---: | ---: | ---: | ---: |
| Unchanged Tokio | 64 B | 708,196 | 711,909 | +0.52% |
| Unchanged Tokio | 4 KiB | 635,515 | 634,457 | −0.17% |
| Core Tokio monitors | 64 B | 646,238 | 655,134 | +1.38% |
| Core Tokio monitors | 4 KiB | 585,320 | 591,851 | +1.12% |

The adapter's 64-byte ranges are 645,485–648,022 before and 650,116–658,440 after. At 4 KiB they overlap: 583,927–594,096 before and 588,105–592,269 after. This is a small promising improvement, not an established universal percentage or proof of server saturation. The closed-loop loopback workload includes client scheduling and networking. The optimization does not remove the separate receive-policy gap with Tokio's ordinary `AsyncRead`.

Two additional counter runs and one cycle profile use the factory build after all throughput controls finish. The counter runs still show two receive calls and one `EAGAIN` per completed echo. They use zero warmup and a three-second measurement, with counters covering startup/shutdown too. The [cycle profile](results/tokio-future-construction/factory/profile.txt) uses one-second warmup and five measured seconds, reports no lost samples, and no longer shows the prior operation-state copy hotspot. Instrumented throughput is not used in the table.

## Provenance and reproduction

[Raw samples and binary hashes](results/tokio-future-construction/aiko.json), [baseline patch](results/tokio-future-construction/baseline/change.patch), [factory patch](results/tokio-future-construction/factory/change.patch), and [measurement scripts](results/tokio-future-construction/scripts) preserve the experiment. Both binaries are built from [`f9cfc4e`](https://github.com/core-rs/core-scheduler/commit/f9cfc4ef3a3d40ff9d4136371e0ad34fb9188e61), the benchmark branch for #32, whose Tokio adapter matches main at `eba10b5`. The baseline patch adds a diagnostic raw-Tokio receive mode; the factory patch adds the private helper change on top. This PR needs no code from #32. The implementation here differs from the measured helper patch only in formatting and its explanatory comment.

Aiko has a Ryzen 9 9950X3D and ran Linux 7.2.8 with Rust 1.99.0. Builds use `CARGO_PROFILE_RELEASE_DEBUG=1 cargo build --release --locked -p core-scheduler-echo-benchmark`, retaining release optimization without LTO or target-CPU overrides. Server threads use CPU 0; the four-worker client uses CPUs 4–7. Those are distinct physical cores in one L3 group. Both sides enable `TCP_NODELAY`. Connections reuse 64 KiB server buffers, complete partial writes, and validate the echoed contents. Builds, benchmarks and profiling run sequentially under `/tmp/core-scheduler-perf.lock`.

The archived script preserves its original Aiko paths. To reproduce it, prepare separate checkouts at that base, apply the corresponding patches, and build with the command above. Adjust the checkout, client, and output paths in `scripts/factory.py` for the new run, keeping `sample.py` and `supervise.py` alongside it. Its output directory must not already exist. Reuse the original client binary when available; otherwise record the new client hash. The supervisor gives elevated access only to the profiler and runs the server as the original user. Inspect the new assembly rather than relying on the archived addresses, which are specific to those binaries.
