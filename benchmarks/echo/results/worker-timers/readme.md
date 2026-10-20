# Worker timer research evidence

The [report](../../worker-timers-research.md) explains this experiment. The diagnostic [candidate patch](candidate.patch) is **not applied** to the production workspace.

The candidate reuses Native’s existing `core-scheduler-timers` queue, lets workers collect its expired values, and gives its next deadline to `SelectorWorker::turn`. It disables the lazy timer thread only for `NativeScheduler::with_drivers`. Default readiness construction retains its timer thread as an architectural control. The diagnostic is deliberately a shared-queue experiment, not a proposed sharded implementation.

Run this from the checkout root to validate the sample matrices, recorded identities, and regenerated tables:

```sh
python3 benchmarks/echo/results/worker-timers/summarize.py
```

| File | Evidence |
| --- | --- |
| [build.json](build.json) | Source, compiler, identical harness hash, candidate patch hash, and optimized executable identities. |
| [provenance.json](provenance.json) | Host, CPU topology, inherited timer slack, and compiler environment overrides. |
| [timers.json](timers.json) | 84 randomized samples: before/integrated io_uring, unchanged dedicated-thread Native, and Tokio timer controls. |
| [echo.json](echo.json) | 54 randomized samples: both Native I/O paths and unchanged pure Tokio, with no application timers. |
| [slack.json](slack.json) | 30 additional samples with 1 ns timer slack inherited by both versions. |
| [stat/results.json](stat/results.json) | Twelve separate instrumented timer runs with whole-process hardware/syscall counters in adjacent `.stat` files. |
| [tables.md](tables.md) | Regenerated medians, ranges, latency, CPU time and thread counts. |
| [summary.json](summary.json) | The same aggregates with numerical precision preserved. |
| [verification.md](verification.md) | Prototype tests and incomplete coverage; production adoption requirements. |
| [scripts](scripts) | Exact build, workload and measurement scripts. |

## Method

All measurements run on Aiko, on separate physical cores of a Ryzen 9950X3D. Timer processes and echo server workers use CPU 0 or CPUs 0–3; the fixed four-worker echo client uses CPUs 4–7. Every build, test, timing and profiling operation acquires `/tmp/core-scheduler-perf.lock` exclusively. The machine’s frequency policy is unchanged.

Both variants build from `130cd4a67e526e80ba9a5e7e7ebbe81494bd903b`, merged PR #17. The baseline additionally contains the identical diagnostic timer binary source; the candidate additionally contains `candidate.patch`. They use locked dependencies, `cargo build --release --locked -p core-scheduler-echo-benchmark --bins`, default release optimization, and no explicit LTO or target-CPU override. `build.json` records Rust 1.99.0 and LLVM 23.1.1. Source changes between main and the parent research commit affect evidence/benchmarks, not production code.

The periodic-timer workload has one second of warmup and three measured seconds. Each task repeatedly requests `Instant::now() + delay`, awaits it, asserts it did not fire early, and records lateness at resumption in its own HDR histogram. Only deadlines inside the measurement window that resume before its end count. The reported rate depends on lateness and the requested delay; it is **not** a maximum timer-queue throughput measurement or a fixed-rate arrival test. Completion and timer registration happen once per iteration. Histograms are allocated before task loops.

CPU time and context-switch counts cover the **whole process**, including warmup, initialization and shutdown. They are not normalized by the measured-only expiry count. Thread counts are one `/proc/PID/task` snapshot at 250 ms; they include the caller thread and workers. Instrumented runs are separate from capacity/latency conclusions. Three uninstrumented repetitions and two counter repetitions are short exploratory measurements, not a sub-percent equivalence proof.

The default timer screen uses the host’s inherited timer slack, observed as 50,000 ns. The matching-slack control sets `PR_SET_TIMERSLACK` to 1 ns in the runner and verifies its own setting before spawning either executable. New worker and timer threads inherit that policy. This is a per-thread/process experiment, with no machine-wide setting changes. The [Linux API documentation](https://man7.org/linux/man-pages/man2/pr_set_timerslack.2const.html) explains wake coalescing and inheritance.

Pure Tokio is a timer policy control, not a comparison at equal precision. These measurements request submillisecond and 1 ms deadlines and observe materially different wake granularity. They do not establish that one queue can process more equally precise timers than another.

Echo uses the existing validated persistent IPv4 loopback workload, one outstanding round trip per connection, `TCP_NODELAY`, reused 64 KiB server buffers, and 64-byte payloads. It uses the frozen client with SHA256 `db6c914ab75011477ec6232e681046b1e75532cbaa1c81663bac984591675cf3`. Pure Tokio remains an unchanged control in both executables. These runs contain no application timers: they measure the cost of checking an empty timer queue, not mixed timer/I/O latency.

## Reproduction

The scripts preserve the exact cache and checkout paths used on Aiko. Copy `scripts/`, `candidate.patch` and `build.json` as appropriate into `~/.cache/core-scheduler-worker-timers/`. Start with a clean detached checkout at `~/Developer/core-rs/core-scheduler-worker-timers` on `130cd4a`, then run `build.py`. It builds and freezes the baseline, applies the candidate, freezes the integrated binaries, and runs Linux Native unit tests. Do not mutate this checkout while building a variant.

Run `run-timers.py`, `run-slack.py` and `stat-timers.py` from the cache. Their output directories must not already exist. The stat script needs passwordless `sudo -n perf` for counters; the workload itself executes as the original user. The generic echo runner uses these arguments:

```sh
research_cache="$HOME/.cache/core-scheduler-worker-timers"
python3 "$HOME/.cache/core-scheduler-single-worker-research/run.py" \
  --output "$research_cache/echo-screen" \
  --build "$research_cache/build.json" \
  --client "$HOME/.cache/core-scheduler-owner-wake-final/client" \
  --repeats 3 \
  --case "before,iouring,$research_cache/before-core-scheduler-echo-benchmark" \
  --case "integrated,iouring,$research_cache/integrated-core-scheduler-echo-benchmark" \
  --case "before,iouring-poll,$research_cache/before-core-scheduler-echo-benchmark" \
  --case "integrated,iouring-poll,$research_cache/integrated-core-scheduler-echo-benchmark" \
  --case "before,tokio,$research_cache/before-core-scheduler-echo-benchmark" \
  --case "integrated,tokio,$research_cache/integrated-core-scheduler-echo-benchmark"
```

Alternatively use the identical archived `run.py` with its adjacent `sample-runner.py`. The default echo workloads are 1/1/64, 1/32/64 and 4/128/64 (workers/connections/bytes). All runners shuffle samples deterministically. Build logs, executables, targets, and temporary/duplicate outputs remain outside the evidence directory in the local and remote research caches.
