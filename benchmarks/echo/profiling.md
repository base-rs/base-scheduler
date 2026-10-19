# Echo performance investigation

The baseline was already optimized. Compiler tuning made small differences; repeated experiments identified larger costs in reactor waiting and readiness bookkeeping. All measurements below used Aiko and source revision `5736301fded46c4e5f11d5387950ae2ee0807b94` from the [initial benchmark](results.md). Experimental patches preserve the isolated investigations. The [implementation follow-up](performance.md) records the final io_uring changes and their combined measurements; the Tokio readiness change remains a prototype.

## Compiler settings

The original runner uses `cargo build --release`. There are no workspace profile overrides. Cargo's release defaults are optimization level 3, disabled debug assertions, 16 codegen units, and local ThinLTO rather than cross-crate LTO. See the [Cargo profile documentation](https://doc.rust-lang.org/cargo/reference/profiles.html).

This investigation built two server binaries from the same source: default release plus `debug=1` for symbols, and release plus `debug=1`, `lto=thin`, and `codegen-units=1`. Verbose compiler invocations confirmed `-C opt-level=3` for both and `-C lto=thin -C codegen-units=1` on the tuned executable. The same baseline client binary was used throughout, so this comparison isolates changing the server build. Neither build requests `target-cpu=native`.

Three repetitions per case, reporting median round trips/second with 64-byte messages:

| Workers | Connections | Backend | Baseline req/s | Experiment req/s | Change |
| ---: | ---: | --- | ---: | ---: | ---: |
| 1 | 1 | tokio | 267,961 | 271,461 | +1.3% |
| 1 | 1 | tokio-adapter | 259,736 | 266,887 | +2.8% |
| 1 | 1 | iouring | 165,568 | 166,847 | +0.8% |
| 1 | 32 | tokio | 692,420 | 712,311 | +2.9% |
| 1 | 32 | tokio-adapter | 581,031 | 590,204 | +1.6% |
| 1 | 32 | iouring | 482,429 | 488,592 | +1.3% |
| 4 | 32 | tokio | 1,738,452 | 1,749,287 | +0.6% |
| 4 | 32 | tokio-adapter | 1,554,804 | 1,580,601 | +1.7% |
| 4 | 32 | iouring | 1,307,909 | 1,303,437 | -0.3% |

These median changes are approximately 0–3%; with three short repetitions, small differences should not be treated as precise improvements. Cross-crate optimization does not explain the original gap. [All compiler comparison samples](results/profiling/compiler.json).

## Reactor and readiness experiments

The [first diagnostic patch](results/profiling/fast-paths.patch) contains two independent backend changes:

- **Native io_uring:** return to the executor after a reactor turn makes progress, before idle-wait housekeeping. The original turn probes the notification socket, rearms the poller's sources, and polls even after processing commands or completions. The experiment keeps those steps for turns without progress.
- **Tokio adapter:** after lazy registration exists, consult `AsyncFd::try_io` first and construct an asynchronous readiness wait only after `WouldBlock`. The original path tries the raw syscall first, then enters `async_io`, which can retry a syscall against stale readiness. The experiment also avoids constructing the async readiness future for immediately successful operations.

These use default release optimization with symbols, without LTO. Each backend is compared against its own baseline in shuffled order:

| Workers | Connections | Backend | Baseline req/s | Experiment req/s | Change |
| ---: | ---: | --- | ---: | ---: | ---: |
| 1 | 1 | tokio-adapter | 251,825 | 262,732 | +4.3% |
| 1 | 1 | iouring | 165,559 | 242,097 | +46.2% |
| 1 | 32 | tokio-adapter | 581,288 | 609,828 | +4.9% |
| 1 | 32 | iouring | 485,828 | 506,020 | +4.2% |
| 4 | 32 | tokio-adapter | 1,541,880 | 1,589,537 | +3.1% |
| 4 | 32 | iouring | 1,271,171 | 1,474,640 | +16.0% |

The io_uring improvement is largest with one connection, where there is little opportunity to amortize poller work across requests. That prototype varied from 210,582 to 243,993 req/s across the three runs, versus 164,619 to 168,106 for the baseline: every run improved, but the median gain is not a precise prediction. The four-worker gain is also substantial. The Tokio change provides a smaller, repeatable direction of improvement. These are separate experiments; their percentages must not be added to LTO gains or to other prototypes. [All fast-path samples](results/profiling/fast-paths.json).

`strace -f -c` corroborates the mechanism. With one worker and 32 connections, the baseline io_uring trace contained 31,425 `epoll_ctl` calls, 7,838 `timerfd_settime` calls, and 7,837 `epoll_pwait` calls. The reactor prototype reduced those to 85, 3, and 2 respectively, while continuing to perform real `io_uring_enter` submissions. These are whole-process counts from separate half-second traced runs, including startup; tracing changes timing and request counts, so their throughput is not used above. [Baseline trace](results/profiling/strace-baseline-iouring-w1-c32-r1.strace), [prototype trace](results/profiling/strace-prototype-iouring-w1-c32-r1.strace).

## Remaining CPU costs

Optimized binaries were sampled separately with `perf record -e cycles:u -F 999 --call-graph dwarf,8192 -m 32`. These are self percentages in sampled profiles, not fractions of wall-clock latency or independent estimates of achievable speedup. Call attribution can include inlined work; the experiments above provide stronger causal evidence than a hotspot alone.

- **Tokio readiness and shutdown:** at one worker/32 connections, Tokio's multi-waiter `Readiness::poll` and `Readiness::drop` account for 10.92% and 14.19% of the adapter's samples. `event_listener` registration, insertion, and removal contribute another 12.35%. The adapter waits on its shared scheduler-close signal for pending operations even when the caller supplies `Cancellation::never()`. Pure Tokio's `TcpStream` uses its specialized polling path. A switch to a single stored waker would need care: our shared socket interface permits concurrent operations. [Adapter profile](results/profiling/perf-baseline-tokio-adapter-w1-c32-r1.data.txt), [pure Tokio profile](results/profiling/perf-baseline-tokio-w1-c32-r1.data.txt).
- **io_uring request completion:** sending and receiving through `futures_channel::oneshot` contributes about 14.7% of the one-worker profile. Every operation constructs a boxed request and a fresh oneshot channel, retains resource/controller ownership, goes through the in-flight map, and schedules completion. The profile establishes completion-channel overhead; it does not isolate the cost of allocation from synchronization. [One-worker profile](results/profiling/perf-baseline-iouring-w1-c32-r1.data.txt).
- **Worker coordination:** `Shared::wake_idle` is 7.25% of the four-worker io_uring profile, and queue stealing, epoch pinning, and task scheduling are also visible. These are candidates for a subsequent targeted experiment after the reactor change. [Four-worker profile](results/profiling/perf-baseline-iouring-w4-c32-r1.data.txt).

## Capacity admission

The current io_uring admission path registers an event listener before checking whether capacity is available. The [separate capacity patch](results/profiling/capacity.patch) acquires immediately when possible and registers a listener only on the full path, with a second state check after registration to preserve wakeups. This was tested independently of the reactor change.

| Workers | Connections | Baseline req/s | Capacity fast path req/s | Change |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 164,060 | 167,447 | +2.1% |
| 1 | 32 | 484,088 | 495,308 | +2.3% |
| 4 | 32 | 1,332,893 | 1,344,292 | +0.9% |

This provides a smaller benefit than the reactor change in the latency-oriented case. It does not eliminate the per-operation completion channel or task scheduling. [All capacity samples](results/profiling/capacity.json).

## Method and reproduction

Each uninstrumented case used one second of warmup and three seconds of measurement, with three shuffled repetitions. Server workers used CPU 0 or CPUs 0–3; four client workers used CPUs 4–7. The client, payload validation, socket flags, and owned-buffer echo workload were held fixed. Both prototype builds passed the benchmark's Linux smoke tests; the capacity patch also passed the io_uring crate's tests. Production changes still need the repository's complete coverage and concurrency review, including cancellation, close, wakeup races, and driver fairness.

Raw JSON includes child user/system CPU time and context-switch counts collected with `getrusage(RUSAGE_CHILDREN)` around reaping each process. Those cover the whole process lifecycle, including startup, warmup, and shutdown; they are not measurement-window-only utilization. For sampled/traced runs, the server totals include profiler/tracer overhead. The existing loopback, closed-loop, and client-placement limitations still apply. No throughput claim is an isolated server-capacity measurement.

The [investigation driver](results/profiling/run.py) is a snapshot of the Linux experiment harness. Process-group cleanup was hardened after collection to terminate server children if a profiler/tracer exits or a run fails; the workload and recorded successful samples are unchanged. It uses the checkout at `~/Developer/core-rs/core-scheduler` and the following build directories:

```sh
CARGO_TARGET_DIR=/tmp/core-scheduler-profile-baseline CARGO_PROFILE_RELEASE_DEBUG=1 cargo build --release --locked -p core-scheduler-echo-benchmark
CARGO_TARGET_DIR=/tmp/core-scheduler-profile-lto CARGO_PROFILE_RELEASE_DEBUG=1 CARGO_PROFILE_RELEASE_LTO=thin CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 cargo build --release --locked -p core-scheduler-echo-benchmark
python3 benchmarks/echo/results/profiling/run.py compare
```

For the fast-path and capacity experiments, create separate worktrees at the recorded source revision, apply the corresponding patch, and build into `/tmp/core-scheduler-profile-prototype` and `/tmp/core-scheduler-profile-capacity`, respectively, with the baseline build flags. Run the driver with `experiments` or `capacity`. `perf` and `strace` collect the baseline diagnostic runs; `strace --variant prototype` collects the revised syscall trace. The driver writes under `/tmp/core-echo-profile`. Only one workload was run at a time, with builds and profile analysis outside timed runs.

The compact profile reports use self samples (`--no-children`), group by symbol, and omit symbols below 1%. For example:

```sh
perf report --stdio --no-children -g none --sort symbol --percent-limit 1 -i /tmp/core-echo-profile/perf-baseline-iouring-w1-c32-r1.data
```

Whitespace is collapsed in the saved text reports. Re-exporting all six recordings with these flags reproduced every saved percentage and symbol row.

## Priorities

The [implementation follow-up](performance.md) addresses the reactor's unnecessary idle-wait work and capacity admission. Remaining candidates include the Tokio readiness fast path, completion-channel reuse, and worker wakeups. Those changes need their own compatibility checks and measurements; completion and worker changes touch ownership and concurrency more deeply.
