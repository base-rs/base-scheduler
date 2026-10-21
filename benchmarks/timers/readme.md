# core-scheduler-timer-benchmark

An experimental reusable timer inspired by C++ `Async::After`, compared with the existing `Clock::sleep_until` operation. This branch is a prototype, rather than a proposal to replace the clock interface or a released public API.

## Design

`Timers` is an optional capability alongside `Clock`. Its associated `Timer` is a reusable, single-waiter resource, constructed through the scheduler to establish compatibility. Construction registers no deadline and selects no worker. Every wait receives its scheduler, absolute deadline and cancellation explicitly:

```rust,ignore
let mut timer = scheduler.timer();
scheduler.timer_wait_until(&mut timer, deadline, &cancellation).await?;
```

A wait begins on first poll. Dropping the wait retains registration state in the resource, so the next wait can replace its deadline and waker. Dropping the resource retires the registration. Cancellation observed after admission disarms it; pre-cancellation leaves existing state unchanged. A completed timer can be reused. Backends reject incompatible scheduler/resource pairings. Retained registrations can issue a wake after the previous wait was abandoned, and retain capacity until the resource is reused or dropped. Forgotten resources may leak capacity, without retaining access to borrowed application memory.

The initial experiment separated two mechanisms:

- `reuse`: retain the backend timer resource, replacing its deadline eagerly.
- `after`: an additional portable wrapper retains an earlier wake when the desired deadline moves later. If that earlier deadline fires, it waits again for the desired deadline. Earlier desired deadlines replace the old registration immediately.

The second experiment moves postponement into the backend's resource. Native already stores an armed deadline; it can retain that registration until the old wake or an earlier desired deadline. Direct can do the same while retaining its map identifier. This needs no new queue algorithm or public policy switch. Tokio already has a fast deadline-extension path in its existing timer entry. The `after` wrapper remains in the benchmark to measure its additional cost, rather than being necessary for Native or Direct.

Native uses its existing reserved worker-local queue, with no timer thread. Registrations remain with their owner when the task moves; replacements and release use existing local routing or that owner's inbox. It does not add an io_uring timer opcode. The `iouring` case uses exactly the same Native timer engine, with per-worker io_uring drivers providing interruptible waits bounded by the earliest deadline. The `native` case uses the default scheduler's thread parking for idle task workers. Direct retains its simple dedicated timer thread. Tokio retains one pinned sleep allocation per resource, reused across waits; its fresh-sleep baseline does not require that allocation.

All timer and cancellation objects are ordinary owned Rust state. No borrowed application pointer is retained by a worker. No timer API requires a caller to prevent future abandonment, or silently obtains its scheduler from TLS.

## Workloads and methodology

Measurements use Aiko: AMD Ryzen 9 9950X3D, 32 logical CPUs, Linux 7.2.6-arch2-1, Rust 1.99.0, LLVM 23.1.1, and locked Tokio 1.53.3. Builds use Cargo's optimized release profile, without added LTO or target-CPU flags. CPU frequency scaling remains `powersave`, with boost enabled; processes are not CPU-pinned. Runs are serialized with `/tmp/core-scheduler-perf.lock`. No builds or tests run concurrently with the measured processes.

Each repeat shuffles backend/workload pairs and runs their three modes consecutively in shuffled order. No separate machine or cross-network traffic is involved. The initial matrix retains the last six of seven repeats, excluding the first following a harness restart. The backend-deferral variant has five repeats. Each process constructs its runtime outside the timed region, then measures task admission, task work and joins. The reported `ns/op` is elapsed wall time divided by completed operation attempts across all tasks. For multiple workers it is aggregate throughput, not individual operation latency. Process CPU time is recorded separately, including timer threads. Small percentage differences should be treated cautiously, especially for Direct's contended mutex and timer-wheel-bound workloads.

All modes use a real, initialized `Cancellation::new()` token. The only difference is timer lifetime and rearming policy.

| Workload | Behavior |
| --- | --- |
| `refresh` | One task polls and drops three million waits, refreshing a deadline 30 seconds ahead. This isolates the hot registration path and does not represent application request throughput. |
| `yield`, one task/worker | 1.5 million refreshes, yielding to the executor after each, so the worker processes its timer queue between refreshes. |
| `yield`, 64 tasks/four workers | 75,000 refreshes per task, yielding after each, permitting task migration and contention. |
| `fixed` | Three million poll/drop attempts for the same distant deadline, separating reuse from deadline movement. |
| `earlier` | Three million attempts with successively earlier distant deadlines. Postponement cannot avoid replacement. |
| `expire` | Repeated one-millisecond waits actually complete: 300 per task with one task/worker, or 200 per task with 128 tasks/four workers. Record expiry overshoot as well as CPU cost. |
| `cycle` | 64 tasks/four workers each execute 200 cycles of 32 refresh/yield attempts followed by an actual one-millisecond expiration. The harness records a refresh which expires during its poll as successful progress. |
| `alloc` | Warm the current worker and resource for 2,048 refreshes, then count allocations during 100,000 attempts on that worker. Background-thread allocations and setup are excluded. |

The one-millisecond expiration rows reflect backend scheduling precision as well as work. Tokio documents [millisecond timer granularity](https://docs.rs/tokio/latest/tokio/time/fn.sleep.html). Its observed wait length must not be interpreted as proportional timer-registration overhead. Absolute throughput from those rows is not a matched-precision backend race.

## Reproduce

```sh
cargo build --release --locked -p core-scheduler-timer-benchmark
flock /tmp/core-scheduler-perf.lock python3 benchmarks/timers/run.py --output results.jsonl
python3 benchmarks/timers/summarize.py results.jsonl
```

A single sample takes backend, mode, workload, tasks, iterations, workers and delay in microseconds:

```sh
target/release/core-scheduler-timer-benchmark native reuse yield 64 75000 4 1000
target/release/core-scheduler-timer-benchmark tokio after alloc 1 100000 1
```

Use `--backends tokio native direct` on macOS. Process CPU time is available on Unix; the experimental harness reports zero on other systems. Performance results in this report are Linux measurements.

## Results

The initial eager-replacement experiment is recorded at `5ff562f`; [its retained samples](results/initial-policy.jsonl) and [full summary](results/initial-policy.md) separate resource reuse from the portable `After` wrapper. Native's eager resource reuse alone did little for continually changing deadlines and was slower for the one-task yield case. Keeping the earlier wake was the useful optimization.

The final backend-deferral implementation was measured at `9a63e72` in the experimental checkout and incorporated here in `b417fa3`. Subsequent commits add test coverage and documentation, rename the capability to `Timers`, and move owner binding into `scheduler.timer()`. The measurements describe the earlier first-wait-binding implementation; the current source retains its queue and rearming policy. [Raw final samples](results/backend-deferral.jsonl) and the [full paired summary](results/backend-deferral.md) preserve all workloads and repeat ranges.

### Refreshing an idle timeout

One task/one worker; medians in wall-clock nanoseconds per refresh attempt. The percentage is the median of each repeat's paired cost reduction, rather than a percentage of unrelated medians.

| Backend | Fresh sleep | Reusable timer with backend policy | Portable `After` wrapper | Reusable cost reduction |
| --- | ---: | ---: | ---: | ---: |
| Tokio adapter | 282.0 | 178.5 | 169.1 | 36.8% |
| Native / thread park | 143.7 | 92.7 | 95.9 | 35.4% |
| Native / io_uring | 148.9 | 91.0 | 94.9 | 38.9% |
| Direct | 400.4 | 143.5 | 148.5 | 64.2% |

The Native backend policy retains the benefit without needing the extra wrapper. Tokio's wrapper still saves approximately another nine nanoseconds in this workload by avoiding some reset calls. That is an implementation opportunity, rather than a reason to expose a public policy choice.

With a yield after each attempt, one-worker Native improves by approximately 15%; its io_uring composition also improves by approximately 15%. With 64 tasks/four workers, their paired median reductions are approximately 32% and 39% respectively, but the ranges are wider. The gain survives actual worker turns; it is not confined to a tight loop which prevents queue flushing.

Successively earlier deadlines provide essentially no Native gain: replacement remains necessary. Waiting on the same fixed deadline is approximately 42% cheaper with the Native resource. These cases distinguish registration reuse from deferral of deadline changes.

### Actual expiration and mixed activity

Actual one-millisecond expirations have essentially unchanged wall-clock completion rates within each backend. The Native mixed refresh/expiry case improves by about 2%; the io_uring case is within about 1%. Most time there is spent waiting for the deadline, so the hot refresh-path improvements do not imply equally large throughput gains for that workload.

Direct's mixed workload is inconclusive and sometimes worse: the paired median for its backend resource is 11% more costly, with individual repeat changes ranging from 27% more costly to 36% less costly. Its contended shared registry and timer thread produce substantial variation. Do not describe this prototype as a universal performance improvement, or interpret its Direct numbers as a reason to complicate that reference implementation.

The one-task expiry overshoot and process CPU work per expiration, using the reusable timer, were:

| Backend | Median p99 expiry overshoot | Process CPU per expiration |
| --- | ---: | ---: |
| Tokio adapter | 1155.8 µs | 6.7 µs |
| Native / thread park | 53.7 µs | 1.4 µs |
| Native / io_uring | 11.0 µs | 1.4 µs |
| Direct | 66.7 µs | 5.5 µs |

These precision differences already exist in the original clock implementations. The io_uring result comes from its timed, interruptible ring wait, not a new dedicated timer submission.

### Allocations and retained state

All modes counted zero allocations on their calling worker during 100,000 warmed refreshes, including the fresh-sleep baseline. The benefit measured here comes from less registration, queue and wake bookkeeping, rather than removing mandatory per-wait allocations. [Allocation samples](results/allocations.jsonl) exclude initial setup and background threads.

On this Linux build, the timer resources occupy 80 bytes for Native, 56 for Direct and 16 for Tokio. The portable `After` wrapper adds 16 bytes. These sizes exclude backend queue storage and Tokio's retained pinned sleep allocation. Native reuses reserved queue storage, but still releases and acquires a generation key on actual replacement/expiry; the prototype does not pretend that it retains a slot forever without bookkeeping.

## Assessment

A reusable timer resource is useful for connection idle timeouts and repeated waits. Keep the existing one-shot sleep operation for occasional delays. Prefer an optional associated timer resource which receives the scheduler and cancellation at each wait. Backend state can handle later deadlines privately; earlier deadlines must remain prompt. This preserves the explicit execution model and avoids making a portable protocol choose a timer-rearming algorithm.

The most important costs to settle before productionizing are retention of queue capacity and an old task's waker after an abandoned wait, the explicit disarm/reset API, and whether caller-pinned Tokio storage is useful enough to avoid the initial box. Conservative instance affinity is established at construction: Native and Direct reject another instance; Tokio permits adapters of the same runtime. A resource can be replaced with a newly constructed timer to disarm it and explicitly select an owner. No network-request throughput gain has been established by this timer-only benchmark.

## Validation

The full workspace tests and Clippy pass on macOS and Linux with all features. Measured region coverage is 18,510/18,510 on macOS and 22,625/22,625 on Linux, with the existing invariant-only unreachable exclusions. Tests cover constructor-established compatibility, first-poll admission, earlier and later deadlines, real expiration, cancellation and pre-cancellation, future/resource abandonment, capacity exhaustion and reuse, closed schedulers, startup failure, incompatible resources, compatible Tokio adapters, and changing polling contexts across executors. The benchmark's modes, workloads, immediate-expiry counting and executable output are exercised too. CI additionally checks Windows and the minimum supported Rust version. This remains an experimental capability; no release is included.
