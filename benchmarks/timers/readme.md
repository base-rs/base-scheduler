# base-scheduler-benchmark-timers

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

The second experiment moves postponement into the backend's resource. Native already stores an armed deadline; it can retain that registration until the old wake or an earlier desired deadline. The since-removed Direct backend could do the same while retaining its map identifier. This needs no new queue algorithm or public policy switch. Tokio already has a fast deadline-extension path in its existing timer entry. The `after` wrapper remains in the benchmark to measure its additional cost, rather than being necessary for Native.

Native uses its existing reserved worker-local queue, with no timer thread. Registrations remain with their owner when the task moves; replacements and release use existing local routing or that owner's inbox. It does not add an io_uring timer opcode. The `iouring` case uses exactly the same Native timer engine, with per-worker io_uring drivers providing interruptible waits bounded by the earliest deadline. The `native` case uses the default scheduler's thread parking for idle task workers. Direct, measured here and since removed, used a simple dedicated timer thread. Tokio retains one pinned sleep allocation per resource, reused across waits; its fresh-sleep baseline does not require that allocation.

All timer and cancellation objects are ordinary owned Rust state. No borrowed application pointer is retained by a worker. No timer API requires a caller to prevent future abandonment, or silently obtains its scheduler from TLS.

## Workloads

All modes use a real, initialized `Cancellation::new()` token. The only difference is timer lifetime and rearming policy.

| Workload | Behavior |
| --- | --- |
| `refresh` | Each task polls and drops a wait per iteration, refreshing a deadline 30 seconds ahead. This isolates the hot registration path and does not represent application request throughput. |
| `yield` | Like `refresh`, yielding to the executor after each attempt, so the worker processes its timer queue between refreshes. With several tasks and workers, tasks can migrate and contend. |
| `fixed` | Poll/drop attempts for the same distant deadline, separating reuse from deadline movement. |
| `earlier`, `earlier-yield` | Attempts with successively earlier distant deadlines, without and with a yield. Postponement cannot avoid replacement. |
| `expire` | Waits of `--delay-us` that actually complete. Record expiry overshoot as well as CPU cost. |
| `cycle` | Each iteration is 32 refresh/yield attempts followed by an actual expiration of `--delay-us`. A refresh which expires during its poll counts as successful progress. |
| `alloc` | Warm the current worker and resource for 2,048 refreshes, then count allocations during the iterations on that worker. Background-thread allocations and setup are excluded. Needs the `allocations` feature. |

The expiration workloads reflect backend scheduling precision as well as work. Tokio documents [millisecond timer granularity](https://docs.rs/tokio/latest/tokio/time/fn.sleep.html). Its observed wait length must not be interpreted as proportional timer-registration overhead. Absolute throughput from those workloads is not a matched-precision backend race.

### Scenarios and periodic timers

Two further families of workloads, added for [stall rescue](../../design/stall-rescue.md), measure the Native scheduler.

The root-thread scenarios drive a Native scheduler from the root thread. Every duration is a multiple of `--unit-us`, one millisecond by default.

| Workload | Behavior |
| --- | --- |
| `root-sleep` | The root thread sleeps one unit per iteration. Root registrations go to each worker in turn. |
| `spawn`, `spawn-pending` | The root thread spawns and joins an empty task per iteration; `spawn-pending` keeps a distant root sleep pending throughout. |
| `task-sleep`, `task-sleep-pending` | Four tasks per worker each sleep one unit per iteration, optionally with a distant root sleep pending. |
| `idle` | No load for one unit per iteration, for idle CPU. |
| `pulse` | A task woken by a timer every 1,000 units, once per iteration. |
| `stall` | A task blocks its worker inside one poll for 200 units while the root thread measures eight sleeps of 10 units, once per iteration. |
| `task-yield` | Sixteen tasks per worker yield once per iteration each, with no timers. |

`periodic`, the stall rescue design's worker-local timer diagnostic, runs `--tasks` tasks that each sleep `--delay-us` again and again, on `native`, or `iouring` on Linux. It measures a window of `--measure-ms` after a warmup of a third of that, and each expiry inside the window is one operation. `--measure-expiries N` instead measures from the start, without a warmup, until every task has recorded `N` expiries, however long that takes. The smoke arguments use it, so that a slow host cannot leave the window empty; its figures are not measurements. `--alignment aligned` rounds each deadline up to a multiple of the delay, so that every task's deadlines fall due together; `--work` adds that many multiply-add steps after each expiry.

## Running

Each process measures one workload and writes one [`base-benchmark`](https://github.com/base-rs/base-benchmark) record to stdout. Options are written `--name value`. Each family of workloads accepts only its own options, and an unknown option, or one of another family, is an error:

| Option | Workloads | Default |
| --- | --- | --- |
| `--workload` | all | `refresh` |
| `--backend` | timer workloads: `tokio`, `native`, or `iouring` on Linux; `periodic`: `native` or `iouring` | `native` |
| `--mode` | timer workloads: `sleep`, `reuse` or `after` | `after` |
| `--tasks` | timer workloads; `periodic` | 1; 1,024 for `periodic` |
| `--iterations` | timer workloads, per task; scenarios | 100,000; 100 for scenarios |
| `--workers` | all | 1; 4 for `periodic` |
| `--delay-us` | timer workloads, which accept zero; `periodic` | 1,000 |
| `--unit-us` | scenarios | 1,000 |
| `--alignment`, `--measure-ms`, `--work` | `periodic` | `unaligned`, 3,000, 0 |
| `--measure-expiries` | `periodic`, in place of `--measure-ms` | none |

```sh
cargo build --release --locked -p base-scheduler-benchmark-timers
target/release/base-scheduler-benchmark-timers --workload yield --backend native --mode reuse --tasks 64 --workers 4 --iterations 75000
target/release/base-scheduler-benchmark-timers --workload stall --workers 2 --iterations 3
target/release/base-scheduler-benchmark-timers --workload periodic --workers 4 --alignment aligned
```

Each record repeats every parameter, and reports for its measured operations `wall_time`, `cpu_time`, `user_cpu_time`, `system_cpu_time` and both context-switch counts; summaries divide them by operations. The timer workloads measure from the first task's admission to the last join, and also report `refresh_expirations`. A scenario measures its iterations after the workers have gone idle. `periodic` measures its window from the root thread, which runs no tasks. `expire`, `cycle`, `root-sleep`, `task-sleep`, `task-sleep-pending`, `stall` and `periodic` measure how late each expiry is, and add `overshoot_p50`, `overshoot_p99` and `overshoot_max` in nanoseconds: quantiles 0.5, 0.99 and 1 of every measured expiry, which summaries report as they are, as `ns p50`, `ns p99` and `ns p100`. `alloc` reports `allocations` and `allocated_bytes`, with the sizes of the timer resource and the `After` wrapper as `timer_size` and `after_size`. Process resource usage comes from `getrusage`, so on other systems than Unix every measurement fails.

Two features add counts at a cost to timing, so take timing from a build without them. Each record lists them in its `features` parameter, so the runner rejects a campaign whose artifacts differ in them. The scenarios that need them declare their own `features`, so preparation builds each tree once more for them and the timed scenarios never run those builds:

- `allocations` enables the `alloc` workload, run by the `alloc` scenario. It replaces the global allocator with a counting one, which slows every allocation.
- `diagnostics` adds the Native scheduler's stall rescue counts (`rescues_stuck_polls`, `rescues_long_batches`, `rescues_leaving_idle`, `rescued_timers`) to every Native record. They cover the whole process, so summaries never divide them by operations. The `stall-rescues` scenario is the control, which must record rescues, and `periodic-rescues` and `periodic-aligned-rescues` count the periodic diagnostics' rescues.

### Comparing builds

[`Cargo.toml`](Cargo.toml) declares scenarios under `[package.metadata.base-benchmark]`. The timer scenarios compare the Tokio adapter, Native, and Native over io_uring in each round, with Tokio as the baseline of the backend comparison. They use the reusable timer (`reuse`), except `refresh-sleep`, `expire` and `expire-contended`, which use fresh sleeps; other modes and configurations run directly with the binary. The root-thread scenarios measure Native alone, and the `periodic` and `periodic-aligned` scenarios compare Native and io_uring with 4 workers and 1,024 tasks. `alloc` and the `-rescues` scenarios run feature builds, described above. Run `cargo bake benchmark:list` for every scenario's resolved arguments, and `cargo bake benchmark:smoke` to run each once with tiny arguments.

To compare revisions, check each out (for example with `git worktree add`), then prepare them, baseline first, and run a scenario:

```sh
cargo bake benchmark:prepare --tree main=../main --tree candidate=.
cargo bake benchmark:run periodic --artifacts target/base-benchmark/prepared-TIME/artifacts.json --cpus 2-5 --rounds 10
```

Preparation builds every tree with the same profile, once for each feature set of the scenarios (`--scenario NAME` prepares only those a scenario needs), and a layout replica: the baseline rebuilt at another path with salted crate metadata, whose differences from the baseline show how much build layout alone moves a result. Rounds run every artifact and variant in a seeded random order under a host-wide lock, and the report compares each with the baseline, warning when a change is within the replica's range. Screening runs are triage; `--mode recorded` adds the prerequisites for acceptance evidence. See the [base-benchmark usage guide](https://github.com/base-rs/base-benchmark/blob/main/context/usage.md) for every option. Scenario arguments come from the current workspace, so every tree must contain this harness. Revisions before it used `run.py` and positional arguments; to compare them, prepare with `--overlay true`, which builds each tree from a copy with this workspace's `benchmarks/timers` in place of its own:

```sh
cargo bake benchmark:prepare --tree old=../old --tree candidate=. --overlay true --scenario refresh
```

Packages the old tree does not lock get this workspace's versions. Where the harness needs a newer version of a package the old tree does lock, the old tree's build uses it too: the manifest lists each such change in `lockfile_changes`, and recorded campaigns refuse them, so an overlaid comparison is a screening run. Every revision before the update to `base-benchmark` 0.2 has such changes, such as `base-benchmark` itself and the `toml` crates its runner uses (which the measured binary does not link), and `signal-hook` 0.4.6 for revisions before the move to `base-benchmark`.

Overshoot is reported as quantiles since that update, and as a mean before it. Summaries treat the two as different metrics, so without `--overlay true` a comparison with an older revision warns that the overshoot metrics are defined differently and compares none of them.

To measure on a dedicated host, such as [hana](https://github.com/base-rs/base-benchmark/blob/main/hosts/hana.md), add `--host HOST` to `prepare`, `run` and `smoke`. Local paths are mirrored on the host with `rsync`, the matching `base-benchmark` command is installed there when it is missing, and outputs are copied back, so `summarize` works locally:

```sh
cargo bake benchmark:prepare --host hana.local --tree main=../main --tree candidate=.
cargo bake benchmark:run periodic --host hana.local --artifacts target/base-benchmark/prepared-TIME/artifacts.json --cpus 2-5 --rounds 10
```

On a host without Bake, the `base-benchmark` command, installed with `cargo install base-benchmark --root DIR`, offers the same operations, with `--manifest-path` and `--package base-scheduler-benchmark-timers`.

Keep a reviewed campaign's directory under `results/<topic>/`, with an [investigation record](https://github.com/base-rs/base-benchmark/blob/main/templates/investigation.md).

## Results

These results were measured with the earlier harness: `run.py`, which ran the timer matrix with positional arguments in shuffled, paired order, and `summarize.py`. Both remain in this repository's history before the move to `base-benchmark`.

Measurements use Aiko: AMD Ryzen 9 9950X3D, 32 logical CPUs, Linux 7.2.6-arch2-1, Rust 1.99.0, LLVM 23.1.1, and locked Tokio 1.53.3. Builds use Cargo's optimized release profile, without added LTO or target-CPU flags. CPU frequency scaling remains `powersave`, with boost enabled; processes are not CPU-pinned. Runs are serialized with `/tmp/base-scheduler-perf.lock`. No builds or tests run concurrently with the measured processes. The harness linked the counting allocator into every build, so these timings include its cost.

Each repeat shuffles backend/workload pairs and runs their three modes consecutively in shuffled order. The initial matrix retains the last six of seven repeats, excluding the first following a harness restart. The backend-deferral variant has five repeats. Each process constructs its runtime outside the timed region, then measures task admission, task work and joins. The reported `ns/op` is elapsed wall time divided by completed operation attempts across all tasks. For multiple workers it is aggregate throughput, not individual operation latency. Process CPU time is recorded separately, including timer threads. Small percentage differences should be treated cautiously, especially for Direct's contended mutex and timer-wheel-bound workloads.

The matrix ran `refresh`, `fixed` and `earlier` with three million attempts on one task and worker, `yield` with 1.5 million on one, and 75,000 per task with 64 tasks on four workers, `expire` with 300 one-millisecond waits on one task, and 200 per task with 128 tasks on four workers, and `cycle` with 200 cycles per task with 64 tasks on four workers. `alloc` counted 100,000 attempts.

The initial eager-replacement experiment is recorded at `5ff562f`; [its retained samples](results/initial-policy.jsonl) and [full summary](results/initial-policy.md) separate resource reuse from the portable `After` wrapper. Native's eager resource reuse alone did little for continually changing deadlines and was slower for the one-task yield case. Keeping the earlier wake was the useful optimization.

The final backend-deferral implementation was measured at `9a63e72` in the experimental checkout and incorporated here in `b417fa3`. Subsequent commits add test coverage and documentation, rename the capability to `Timers`, and move owner binding into `scheduler.timer()`. The measurements describe the earlier first-wait-binding implementation; the current source retains its queue and rearming policy. [Raw final samples](results/backend-deferral.jsonl) and the [full paired summary](results/backend-deferral.md) preserve all workloads and repeat ranges.

The stall rescue counts of every stage 2 acceptance workload, measured with the `scenario` and `periodic` modes and `diagnostics` builds, are in [results/rescue-counts](results/rescue-counts/readme.md), and what the rescued workers were doing, with the wake latency of idle workers, in [results/rescue-causes](results/rescue-causes/readme.md). What the monitor costs, separated from the rest of the change, and how rescues behave under oversubscription, is in [results/rescue-costs](results/rescue-costs/readme.md). Why the unaligned diagnostic's p50 rose after the timer queue rework, with a spread-phase diagnostic that removes its feedback, is in [results/timer-batching](results/timer-batching/readme.md).

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
