# Current socket path profiles

This investigation profiles the native direct-syscall/ring-readiness path, owned io_uring transfers, and pure Tokio at `a0209b7211a2ce62306be1659bf766f0e4bc87a3`. It follows the [readiness performance comparison](ring-readiness-performance.md). No scheduler implementation changes are included.

## Uninstrumented controls

Three randomized repetitions per case, using the same optimized server executable and fixed client. Each run has one second of warmup and three seconds of measurement. Throughput and latency describe that measurement window. Server CPU time covers the whole process lifecycle, including warmup, startup, and shutdown; it is not measurement-window CPU utilization. Mean latency and CPU columns are medians of per-run values.

| Workers / connections | Bytes | Backend | Median req/s (min–max) | Mean RTT, µs | Server CPU seconds, user / system |
| --- | ---: | --- | ---: | ---: | ---: |
| 1 / 1 | 64 | `tokio` | 272,060 (271,354–275,439) | 3.65 | 0.317 / 1.307 |
| 1 / 1 | 64 | `iouring` | 236,395 (234,861–237,642) | 4.20 | 0.905 / 2.439 |
| 1 / 1 | 64 | `iouring-poll` | 183,125 (175,426–183,500) | 5.41 | 0.442 / 1.698 |
| 1 / 1 | 4096 | `tokio` | 234,644 (233,774–241,683) | 4.20 | 0.281 / 1.326 |
| 1 / 1 | 4096 | `iouring` | 186,953 (186,603–193,469) | 5.28 | 0.778 / 2.316 |
| 1 / 1 | 4096 | `iouring-poll` | 210,883 (208,769–211,059) | 4.68 | 0.470 / 2.068 |
| 1 / 32 | 64 | `tokio` | 697,342 (693,303–704,499) | 45.82 | 0.240 / 2.295 |
| 1 / 32 | 64 | `iouring` | 516,766 (514,274–518,245) | 61.85 | 0.933 / 1.951 |
| 1 / 32 | 64 | `iouring-poll` | 555,618 (555,366–558,129) | 57.53 | 0.551 / 2.244 |
| 4 / 32 | 64 | `tokio` | 1,723,336 (1,717,376–1,764,540) | 18.52 | 1.829 / 6.978 |
| 4 / 32 | 64 | `iouring` | 1,486,617 (1,479,785–1,488,875) | 21.48 | 5.569 / 7.041 |
| 4 / 32 | 64 | `iouring-poll` | 1,697,242 (1,689,961–1,700,173) | 18.81 | 3.002 / 8.792 |

These controls reproduce the main ordering: native readiness approaches Tokio with four workers, but its single-worker cases remain behind. The native readiness single-connection result also depends strongly on payload size. Its four-worker median throughput is close to Tokio while consuming more server CPU over the run, so throughput alone does not establish equivalent efficiency.

## CPU profiles

The profiles identify two different targets: the idle path for single-connection latency, and request/task bookkeeping for concurrent workloads. They do not identify missing compiler optimization: these are release builds of the same server executable.

The supplementary `cycles` recordings include userspace and kernel execution in the server process tree. For one connection and 64-byte messages, visible kernel self rows account for about 80% of native readiness sampled cycles. `fdget`, `fget`, and `fput` together account for 17.7% of total sampled cycles, compared with 5.1% for Tokio. These functions serve several syscalls; their cost cannot all be attributed to readiness registration.

A [representative native readiness callgraph](results/readiness-profiling/kernel/perf-iouring-poll-w1-c1-b64-r1.callgraph.txt) attributes 13.6% inclusive cycles to `epoll_pwait`, 10.1% to `epoll_ctl`, and 4.7% to `timerfd_settime`. These are CPU costs within those call paths, not time spent asleep. The [reactor's idle path](../../crates/iouring/src/io_uring_selector.rs) drains its notification socket, rearms both sources, and waits through `polling`. This supports investigating that extra wait machinery before optimizing allocation for the single-connection case.

With 32 connections, native spends much less time in that idle path. Request lifecycle and executor coordination become more useful targets. The following are medians of self percentages from the two full `cycles` recordings, relative to all sampled cycles in each recording:

| Symbol group | Owned, 1 worker | Readiness, 1 worker | Owned, 4 workers | Readiness, 4 workers |
| --- | ---: | ---: | ---: | ---: |
| Completion oneshot, including drop glue | 4.08% | 2.44% | 3.83% | 2.25% |
| Capacity acquisition and permit release | 2.98% | 0.98% | 3.05% | 0.81% |
| `event_listener` | 2.80% | 1.50% | 3.79% | 1.61% |
| Crossbeam deque and epoch operations | 0.80% | 0.37% | 5.84% | 3.60% |
| `Shared::wake_idle` | 0.58% | 0.30% | 3.71% | 1.14% |

The initial three `cycles:u` repetitions also consistently put oneshot, admission, and event notification among native's prominent symbols. Their relative percentages are higher because the event excludes most kernel execution. Some recorded instruction pointers nevertheless appear as unresolved kernel addresses, so those reports are retained as supporting evidence, not treated as a clean userspace/kernel split.

[Readiness waits](../../crates/iouring/src/io_uring_selector/readiness.rs) currently acquire capacity, allocate a request and oneshot state, retain the socket, publish the request, then retire it through a completion. Ready syscalls already bypass this. The profiles support reducing the whole waiting-request lifecycle; they do **not** establish that allocator calls alone explain the gap, or that an ownership/API change is necessary.

The increased Crossbeam and `wake_idle` shares at four workers support examining work searches, stealing, and wake coordination. They do not prove cache contention by themselves; that would require a targeted experiment or hardware evidence. Networking, packet management, locking, and descriptor lookups also occupy substantial kernel time in every backend.

## Syscall evidence

The [final counter collection](results/readiness-profiling/syscalls/aiko.json) counts both `epoll_wait` and `epoll_pwait`; the first supplementary collection omitted `epoll_wait`, which Tokio uses. The table shows medians of per-run counts divided by completed requests, plus the throughput range relative to the uninstrumented control median. Values displayed as `0.000` can include startup calls.

| Workers / connections | Bytes | Backend | Counter-run throughput / control median | `io_uring_enter` / request | `epoll_ctl` / request | Epoll waits / request |
| --- | ---: | --- | ---: | ---: | ---: | ---: |
| 1 / 1 | 64 | `tokio` | 61.0% (60.9–61.1%) | 0.000 | 0.000 | 1.033 |
| 1 / 1 | 64 | `iouring` | 93.7% (93.4–94.0%) | 6.972 | 4.053 | 1.056 |
| 1 / 1 | 64 | `iouring-poll` | 85.4% (85.1–85.6%) | 3.016 | 5.000 | 2.000 |
| 1 / 1 | 4096 | `tokio` | 74.9% (74.2–75.6%) | 0.000 | 0.000 | 1.033 |
| 1 / 1 | 4096 | `iouring` | 99.3% (97.9–100.6%) | 6.972 | 4.346 | 1.348 |
| 1 / 1 | 4096 | `iouring-poll` | 66.7% (64.8–68.6%) | 3.016 | 5.000 | 2.000 |
| 1 / 32 | 64 | `tokio` | 96.3% (95.7–96.8%) | 0.000 | 0.000 | 0.048 |
| 1 / 32 | 64 | `iouring` | 100.1% (99.8–100.5%) | 0.128 | 0.000 | 0.000 |
| 1 / 32 | 64 | `iouring-poll` | 96.0% (95.9–96.1%) | 0.065 | 0.000 | 0.000 |
| 4 / 32 | 64 | `tokio` | 89.4% (89.1–89.6%) | 0.000 | 0.000 | 0.083 |
| 4 / 32 | 64 | `iouring` | 99.8% (98.7–100.9%) | 0.657 | 0.066 | 0.017 |
| 4 / 32 | 64 | `iouring-poll` | 99.4% (98.9–99.8%) | 0.430 | 0.106 | 0.029 |

For one connection, native's instrumented runs repeatedly enter the ring and its surrounding wait machinery. With 32 connections on one worker, the ring batches submissions and almost never sleeps: epoll rearming is therefore not a good explanation for that case's remaining gap. The full-cycle callgraphs and source support these two distinct paths.

Counters are still intrusive: single-connection Tokio falls to 61% of control throughput, and native readiness with 4 KiB messages falls to 67%. The four-worker Tokio counter run also drops about 11%. Consequently, do not compare instrumented backend throughput as a new ranking or assume these are exact uninstrumented syscall rates. Counter runtime is 100% for every retained event; multiplexing is not the explanation for the perturbation.

`recvfrom` and `sendto` counts include native's Unix notification socket, so they cannot be read as payload-I/O counts. Owned ring transfers are not separate userspace receive/send syscalls. Counts also include startup and shutdown; there is no warmup in this counter phase.

The earlier `strace` summaries demonstrate a stronger observer effect. In the single-connection 64-byte readiness trace, 44,558 `recvfrom` calls produced just four errors, and there were only 11 `io_uring_enter` calls. Tracing slowed the server enough that almost all reads were ready. That trace therefore cannot diagnose the normal waiting path or prove that ring waits are inexpensive. The original trace is retained rather than silently discarded.

## Next experiments

1. **Simplify the idle ring wait.** Prototype sleeping directly on the ring, with explicit wake and timeout sources, or otherwise remove redundant rearming/timer setup. The single-connection profile is the clearest evidence for this experiment. Preserve external task wakes, timer deadlines, cancellation, and shutdown; do not introduce a lost-wakeup race. Compare both latency and CPU use.
2. **Reuse waiting-request state.** Test reusable ring-local request slots and completion state, reducing per-wait oneshot, admission/notification, and request-retirement work. Measure the 32-connection cases first. Keep owned operations safe after future drop, and retain full completion-batch publication before tasks resume. This is an implementation experiment, not a reason to weaken the interface's ownership contract.
3. **Reduce unnecessary worker coordination.** Profile and test local wake batching, empty queue searches, and steal frequency independently. Keep per-worker rings and task migration; first demonstrate that a proposed reduction preserves progress and improves CPU efficiency.

A hybrid owned receive/direct nonblocking send remains worth measuring, but this collection does not isolate its benefit. Multishot operations, registered resources, polling threads, and broader compiler changes should follow simpler experiments with evidence. None of the percentages above is a predicted speedup, and this profile does not establish that native will outperform Tokio.

Start with the idle-wait experiment as a separate change, then repeat the same uninstrumented matrix before accepting it. No scheduler optimization was implemented during this investigation.

## Method and retained evidence

The initial [runner](results/readiness-profiling/run.py) collection contains three sequential phases: 36 uninstrumented controls, 36 CPU profiles, and 12 syscall summaries. Backend and case order are independently shuffled within each phase with seed `20261023` plus the phase index. The [raw measurements](results/readiness-profiling/aiko.json) retain every sample, hashes, machine/tool versions, CPU placement, and process user/system time and context switches.

All three server modes use one executable built with `CARGO_PROFILE_RELEASE_DEBUG=1 cargo build --release --locked -p core-scheduler-echo-benchmark`, retaining the default release optimization settings. This enables symbols without changing optimization level or requesting cross-crate LTO or `target-cpu=native`. The fixed client remains the executable from `5736301fded46c4e5f11d5387950ae2ee0807b94`; its hash matches the preceding comparisons. These controls establish performance for the profiled server build, rather than using historical rates from a differently linked executable.

The host is Aiko (Ryzen 9 9950X3D, Linux 7.2.6, Rust 1.99.0). Server threads inherit CPU 0 or CPUs 0–3; the four-worker client uses CPUs 4–7. These are separate physical cores sharing one last-level cache. No builds or other benchmark/profiler workloads run alongside collection. Profile report generation starts only after all timed workloads finish.

Initial CPU recordings use `perf record -e cycles:u -F 999 --call-graph dwarf,8192 -m 32`, with one second of warmup and five seconds of measurement. All 36 report zero lost samples. Host `perf_event_paranoid=2` prevents unprivileged kernel sampling; it was left unchanged. Self percentages describe the recorded cycle distribution, not wall-clock time or achievable speedups. Inlining can attribute several operations to their containing function. The text reports retain symbols at or above 0.1%; totals grouped from these reports exclude smaller entries. Inclusive callgraph percentages overlap and must not be added. Instrumented throughput is retained for diagnostics but is not used for performance comparisons.

`strace -f -c -S calls` runs separately, with no warmup and a half-second measurement. Counts cover the full process, including startup and shutdown. Dividing by measured completed requests provides only an approximate count per round trip; startup work and boundary requests remain. Tracing changes scheduling and batching, especially with multiple connections, so these are observations of traced runs, not exact syscall rates for the uninstrumented benchmark. I/O performed inside io_uring is not a separate userspace `recv` or `send` syscall.

A supplementary collection uses Aiko's existing process-scoped sudo access for `perf`: 24 syscall-counter runs followed by 24 full `cycles` profiles, with two independently shuffled repetitions per case. The server still runs as `samuel`; only the profiler is privileged. These profiles cover process-tree user and kernel execution, including in-context network processing, but not all work performed by independent kernel workers or other processes. One owned four-worker recording reports two lost samples out of roughly 23,000; the other 23 report zero. The [supplementary JSON](results/readiness-profiling/kernel/aiko.json) and reports retain this evidence.

Full-cycle sampling also changes some cases: median native readiness throughput is 99.1% of control for one connection at 64 bytes, but 79.2% at 4 KiB; single-connection Tokio at 64 bytes reaches 71.5%. All 32-connection profile medians are within 98.1–100.7% of controls. Profiles therefore describe where the instrumented process spends cycles, rather than an exact decomposition of the uninstrumented latency gap.

A final 24-run counter collection adds the initially omitted `epoll_wait` event and uses the hardened supervisor. All stat runs have zero warmup and three seconds of measurement. Counts and task-clock include process startup/shutdown and wrappers. Recorded `server_cpu` for instrumented runs also includes profiler/supervisor overhead; only uninstrumented controls supply the CPU table above. Full counter runtime and running percentage are retained in each `.stat` file. Both counter collections report 100% running for every event, with no unsupported counters.

The initial collection used runner commit `9c8bd5e`; the supplementary collection used `b4da002`; the final counter collection used `00eec7c`. The current runner uses a privileged [supervisor](results/readiness-profiling/supervise.py) with a control pipe and deadline, and `setpriv` to run the workload unprivileged in the profiler's process group. Cleanup checks on Aiko cover successful status propagation, caller departure, timeout, and a wrapper exiting before its child. This cleanup change did not modify the server executable or the completed recordings.

The raw binary recordings remain on Aiko in `/tmp/core-readiness-profile-a0209b7/` and `/tmp/core-readiness-kernel-profile-a0209b7/`; the final counters are in `/tmp/core-readiness-syscalls-a0209b7/`. Self reports from every repetition, inclusive callgraphs from repetition one, and syscall summaries/counters are retained under `results/readiness-profiling/`. Self-report horizontal whitespace and callgraph trailing/column padding are normalized for storage; callgraph indentation is preserved. The [summarizer](results/readiness-profiling/summarize.py) reproduces control medians, self-symbol groups, normalized counters, and lost-sample totals from these artifacts. The shared [benchmark methodology](readme.md) describes the closed-loop workload and its limits; none of these measurements establishes isolated server capacity or client headroom.

Build the server at the recorded source revision, retain the fixed client, then run into a new output directory:

```sh
python3 benchmarks/echo/results/readiness-profiling/run.py \
  --server /tmp/core-ring-readiness-profile/release/core-scheduler-echo-benchmark \
  --client /tmp/core-scheduler-profile-baseline/release/core-scheduler-echo-benchmark \
  --source a0209b7211a2ce62306be1659bf766f0e4bc87a3 \
  --output /tmp/core-readiness-profile-a0209b7
```

For the supplementary collection, use a new output directory and append `--repeats 2 --modes stat perf --perf-event cycles --privileged-perf`. For the final counter-only comparison, use `--repeats 2 --modes stat --privileged-perf`. Copy `supervise.py` alongside the runner when running outside the checkout. Generate aggregate evidence with:

```sh
python3 benchmarks/echo/results/readiness-profiling/summarize.py > /tmp/readiness-profile-summary.json
```
