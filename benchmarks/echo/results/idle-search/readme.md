# Skip quiescent work-stealing searches

Native keeps both ready generations and each worker's incoming queue stealable. This change skips searching other workers when there is only one worker or the existing outstanding-runnable count is zero. It adds an Acquire read at an exhausted-local-queue boundary, with no new counter updates or per-task storage. Local incoming work is checked first. Runnable ownership includes queued work and active polls, so a running task keeps stealing available for uneven work.

The enqueue, idle publication, SeqCst fence, final queue recheck, and notification protocol stays intact. An initial empty search does not authorize parking on its own. Work racing the final search must still be observed by the recheck or retain a wake through the existing notification protocol.

## Echo comparison

[confirm.json](confirm.json) preserves 168 uninstrumented samples comparing the existing Native runtime (`current`), a disabled-stealing diagnostic (`no-steal`), the guard (`quiescent`), and pure Tokio. Source starts at `e3b67a578cf8b7e57b0b3e8623a0f40439fe7c33`. Only Native worker search and its existing counter accessor change between Current and Quiescent; socket readiness, io_uring policy, task admission, and the public interface stay the same.

Aiko has an AMD Ryzen 9 9950X3D with 16 physical cores and 32 hardware threads. It runs Linux `7.2.8-arch1-2`, Rust `1.99.0`, and LLVM `23.1.1`; [environment.txt](environment.txt) records its topology and toolchain. All servers use optimized Cargo release builds with debug information, without LTO or target-CPU overrides. Measurements hold `/tmp/core-scheduler-perf.lock` exclusively. Server workers share CPUs 0–3, restricted to the requested worker count; the separate frozen four-worker validating client uses CPUs 4–7. Workers are not individually pinned. Each sample has one second of warmup and three seconds of measurement. The runner shuffles cases, variants, and three repetitions per cell, and records executable and client hashes.

The adapters use the same generic borrowed or owned echo loops. Pure Tokio uses ordinary borrowed `TcpStream` reads and writes. Native owned I/O can use independently retiring completion operations, so the two Native paths are reported separately against the same Tokio control. These loopback, closed-loop throughput and RTT measurements do not establish application latency under an external arrival rate.

| Workers | Connections | Bytes | Borrowed gain | Owned gain | Borrowed vs Tokio | Owned vs Tokio |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| 1 | 1 | 64 | +0.7% | −1.4% | +0.1% | −15.1% |
| 1 | 1 | 4096 | −0.1% | −0.7% | −0.7% | −3.9% |
| 1 | 32 | 64 | −0.5% | +1.0% | −9.9% | −7.7% |
| 1 | 32 | 4096 | +0.7% | +0.9% | −9.1% | −7.0% |
| 4 | 1 | 64 | +9.3% | +1.5% | −1.2% | −13.9% |
| 4 | 1 | 4096 | +0.8% | −1.3% | −0.1% | −2.1% |
| 4 | 32 | 64 | +0.0% | +0.5% | +0.5% | −0.5% |
| 4 | 32 | 4096 | −0.5% | +0.5% | −1.1% | −1.6% |

“Gain” compares Quiescent with Current in the same cell; rates are medians of three repetitions. Changes near 1–2% are not established wins. Negative “vs Tokio” values mean lower throughput. The clear gain is the four-worker, single-connection, 64-byte borrowed case:

| Variant | Requests/second | Median sample RTT P99 |
| --- | ---: | ---: |
| Current | 248,679 | 5.643 µs |
| No stealing | 274,948 | 4.351 µs |
| Quiescent | 271,791 | 4.463 µs |
| Tokio | 275,075 | 4.571 µs |

Current ranges from 229,541 to 253,772 requests/second across repetitions, versus 268,985 to 274,607 for Quiescent. The remaining one-worker, many-connection and small-message owned-I/O gaps are separate performance work; disabling stealing does not remove them.

## Uneven work

The private diagnostic [fixture](stealing-benchmark.rs) lets 128 tasks suspend on individual channels, then routes their next wake to one worker. That worker stays inside a poll for 20 ms while the tasks are woken externally. Each task performs 100 µs of CPU work and records its start/completion time and executing worker. The busy worker does not wait for result collection, allowing other workers to complete the burst while it is occupied. This intentionally bypasses round-robin admission to isolate stealing; it is not an HTTP latency model.

[confirm-burst.json](confirm-burst.json) records three processes per variant and worker count, with one warmup round followed by ten measured rounds per process. Each value below is the median of the measured rounds' 99th-percentile task completion latencies.

| Workers | Current completion P99 | No stealing | Quiescent |
| --- | ---: | ---: | ---: |
| 4 | 4.477 ms | 32.825 ms | 4.325 ms |
| 16 | 1.769 ms | 32.884 ms | 1.815 ms |
| 32 | 0.629 ms | 32.861 ms | 0.708 ms |

Process-to-process variation is substantial at 32 workers. A [separate follow-up](many-workers-burst.json) runs ten processes per variant and records 100 measured rounds each. Current measures 0.634 ms and Quiescent 0.635 ms, a 0.2% difference; the earlier apparent regression does not repeat. Both use a median of 25 workers and execute every measured task away from the busy owner. Disabled stealing uses only that owner and takes 32.872 ms. Every dataset preserves all per-task times, worker distributions, and executable hashes; burst samples are compacted onto individual JSON lines without changing their contents.

## Counters

[cpu.json](cpu.json) contains 30 separately instrumented samples comparing Current, disabled stealing, empty-victim checks, Quiescent, and Tokio with four workers and one/32 connections. Profiling changes timing substantially in the single-connection case; these rates are mechanism checks, and the throughput conclusions above use only uninstrumented samples.

With one connection, Current executes 4,839 user-space instructions per request versus 3,318 for Quiescent, a 31.4% reduction. Disabled stealing measures 3,298 and empty checks 3,689. Quiescent reduces user-space cycles by 21.3%, while kernel instructions remain at about 19,960 per request. With 32 connections, its user-space instruction change is only −0.5% and kernel instruction change +0.1%.

The nine [syscall and tracepoint samples](work.json) compare Current, disabled stealing, and Quiescent with four workers and one connection. All retain approximately one `io_uring_enter`, one io_uring task-work run, and one CQE per request. Writes and futex calls are negligible. The measured improvement comes from less user-space scheduler bookkeeping, rather than fewer I/O syscalls.

## Reproduction

[reproduce.sh](reproduce.sh) builds frozen variants and runs the echo, uneven-burst, and counter comparisons. Give it a clean disposable worktree at the recorded base revision and a new output directory. It changes that worktree's Native sources. The [prototype patches](prototypes) preserve the original interventions, including a test-only fixture declaration; the production change omits that declaration and adds separate wake-boundary regression tests. The disabled-stealing variant is a diagnostic control, rather than a conforming replacement for work-stealing behavior.

On Linux with Cargo, Python 3, `taskset`, and `flock` installed:

```sh
git worktree add --detach /tmp/core-scheduler-idle-search e3b67a578cf8b7e57b0b3e8623a0f40439fe7c33
bash benchmarks/echo/results/idle-search/reproduce.sh /tmp/core-scheduler-idle-search "$HOME/.cache/core-scheduler-idle-search-reproduction"
```

The default frozen client is the newly built Current executable, reused for every server. Supply a third argument to reuse an existing client; the original measurements use the client path and SHA-256 recorded in each dataset. Counter collection additionally requires noninteractive `sudo perf stat` access and the recorded io_uring/syscall tracepoints. CPU lists and worker counts target Aiko's topology; adapt and record them for another host. These scripts preserve the measurement protocol and source interventions; binary hashes and results can vary with toolchain, checkout path, and host.
