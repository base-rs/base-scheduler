# Native Linux epoll measurements

The worker-owned Linux backend removes the general poller's waiter coordination, normalized event staging, unconditional notification draining and notification rearming. Each worker dispatches directly from one reusable native event vector. Persistent edge registrations retain the existing stable pointers, batch-boundary retirement and failed-registration quarantine.

Zero and indefinite waits use `epoll_pwait` directly. Positive finite waits arm a monotonic timerfd with nanosecond precision and disarm it after success, interruption or failure. Internal timer records are filtered before pointer decoding; notification records are drained during dispatch. Native eventfd or timerfd creation failure selects the original `polling` backend before socket registration. The implementation does not probe `epoll_pwait2` or round sub-millisecond deadlines.

## Final implementation

The final implementation reduces full HTTP CPU per request by **18.24% at one connection** and **2.96% at 16 connections**. At 64 connections it uses **0.43% more CPU per request**, with essentially flat paired throughput. The sparse-traffic CPU improvement survives the compatibility hardening; this is not an across-the-board throughput win.

| Connections | Before req/s | After req/s | Paired throughput | Paired CPU/request |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 31,164 | 30,813 | -0.42% | -18.24% |
| 16 | 300,270 | 305,166 | +1.72% | -2.96% |
| 64 | 357,365 | 359,236 | +0.00% | +0.43% |

All 30 samples completed without wrk-reported errors (56,256,713 requests). CPU improved in all five pairs at one and 16 connections. At 64 it increased in all five pairs, ranging from +0.13% to +1.87%. One-connection throughput decreased in all pairs (median −0.42%); 16-connection throughput improved in all pairs (median +1.72%). Tail latency shows no stable improvement. These measurements support a low-concurrency CPU-efficiency tradeoff, with a small measured busy-case CPU cost.

See the [final samples](results/native-epoll-linux-20261027/final/samples.jsonl), [summary](results/native-epoll-linux-20261027/final/summary.json), [environment](results/native-epoll-linux-20261027/final/environment.json), and [complete final logs](results/native-epoll-linux-20261027/final-evidence.tar.gz).

These are five randomized adjacent pairs per condition, each with a one-second warmup and eight measured seconds. The workload is the downstream, unreleased Base HTTP/1 hello server with persistent connections and one application worker. Both binaries use the same HTTP sources, lockfile, Rust 1.98.1 and release profile. The baseline scheduler is `549603509de7577f76ccce9a8a261be743d07af4`; the candidate is identified by the [source patch](results/native-epoll-linux-20261027/final-source.patch), per-source hashes in the environment, and binary SHA-256. The saved patch includes validation changes; generated documentation and measurement files do not affect the executable.

Measurements ran in a six-vCPU ARM64 Podman AppleHV VM with 8 GiB RAM and Linux 6.12.13. The server process and all its helper threads were pinned to guest CPU 0, wrk to CPUs 2–5, and the controller to CPU 1. No builds or profilers ran during timing. Host physical CPU placement and frequency remain uncontrolled. `hana.local` did not resolve, so this is a VM comparison rather than a bare-metal result.

Linux `/proc/PID/stat` provides whole-process user and system CPU at 100 ticks/second; CPU per request includes both. Each sample validates complete responses and connection reuse before timing. Timed wrk traffic does not validate bodies. Expected connection resets during wrk shutdown can appear in retained server stderr.

Throughput and CPU changes are medians of matched ratios; before/after rates are separate medians, so their ratio need not equal the displayed paired change. Absolute rates should not be pooled across cohorts. These results do not establish multi-worker scaling, TLS, large-body, wake-storm or connection-churn performance.

## Earlier prototype

The earlier `epoll_pwait2` prototype is preserved separately and must not be treated as the final implementation. Its five-pair sweep found full HTTP CPU/request changes of −17.05%, −4.18% and −0.93% at 1, 16 and 64 connections. A longer 64-connection confirmation found −0.11%, with no meaningful high-concurrency HTTP gain. The separate raw transport diagnostic reduced CPU/request at all three concurrency levels. These cohorts contained 80 samples and 208,758,124 completed requests, with no wrk-reported errors.

The [complete prototype archive](results/native-epoll-linux-20261027/prototype-evidence.tar.gz) preserves its source snapshots, patch, runners, environment, per-sample logs, syscall tracing and hash manifest. The [main summary](results/native-epoll-linux-20261027/prototype/main/summary.json) and [confirmation summary](results/native-epoll-linux-20261027/prototype/confirmation/summary.json) are also available directly. The prototype's syscall counts and precise-timeout behavior are not claims about the final timerfd implementation.

## Validation and reproduction

Full workspace, all-target, all-feature coverage passed on Linux and macOS at 100% of measured source regions. Regression tests exercise real eventfd saturation and socket backpressure, independent duplex waiters, cancellation, closure, retained batches and failed-registration quarantine. Thread-local fault injection covers native syscall and compatibility-constructor/wait failures while delegating successful operations to real implementations. See [validation metadata](results/native-epoll-linux-20261027/validation.json) and [logs](results/native-epoll-linux-20261027/validation-logs.tar.gz).

To reproduce the final comparison, unpack the prototype archive, place its HTTP crates beside a scheduler checkout, and build the saved hello benchmark with its lockfile and release profile at the baseline revision. Save the executable as `binaries/polling`. Apply `final-source.patch`, rebuild with the same toolchain and profile, and save it as `binaries/native-final`. Use the original archived `state.json` for source provenance. Place the [final runner](results/native-epoll-linux-20261027/pr_bench.py) beside those sibling crates, `state.json`, and `binaries/`, with Python 3, wrk and taskset installed. Select an unused output directory:

```sh
taskset -c 1 python3 pr_bench.py --output results/pr-final --repetitions 5 --duration 8 --concurrency 1 16 64 --workloads http
```

The [artifact manifest](results/native-epoll-linux-20261027/hashes.json) identifies the published evidence. The runner records both binary hashes, compiler and CPU information, source hashes, raw samples and per-sample logs.
