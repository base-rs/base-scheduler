# Stall rescue costs

These results separate what [stall rescue](../../../../design/stall-rescue.md) costs from the rest of its change, and check whether its rescues cost more when workers compete for CPUs. They follow the [acceptance run](../../../echo/results/stall-rescue/readme.md), whose largest regression was the readiness selector's p50 lateness in the 1,024-timer periodic diagnostic. That run had no build with stall rescue switched off.

Three builds were compared:

- **before:** `main` at `7b158df`, before stall rescue;
- **on:** `main` at `793cbbf`;
- **off:** `793cbbf` with [off.patch](off.patch), which makes `stall_rescue` default to off. It keeps every other part of the change, including the lockable timer queues and published deadlines, but starts no monitor.

The periodic diagnostic and pulse are the stall rescue scripts as run for the acceptance results (`timer-diagnostic.rs` and `rb.rs` in [`stall-rescue/scripts`](../../../echo/results/stall-rescue/scripts/)), built against each tree.

Hana: Linux 7.2.14, Intel Xeon E3-1240 v6 (4 cores, 8 threads), `powersave` governor, rustc 1.99.1, release builds, unpinned unless stated. Every run held `/tmp/base-scheduler-perf.lock`, and nothing else of ours ran on the host. The drivers take build names `m2` (before), `now` (on) and `nowoff` (off), for binaries under `diag-{build}` and `rb-{build}`.

| File | Contents |
| --- | --- |
| [costs.py](costs.py), [costs.jsonl](costs.jsonl) | `costs.py m2,now,nowoff 15 costs.jsonl`: the periodic diagnostic on both selectors, aligned and unaligned, and pulse, with each run's per-thread CPU time and context switches. |
| [oversub.py](oversub.py), [oversub.jsonl](oversub.jsonl) | `oversub.py m2,now,nowoff 9 COUNTS_BINARY oversub.jsonl`: the periodic diagnostic with 8 workers, with 8 workers beside four busy-loop processes, and with 4 workers pinned to two CPUs. Its rescue counts come from the timer benchmark's `periodic` mode built with `diagnostics` at [rescue-counts](../rescue-counts/readme.md)' revision, before decision 16 changed the labels. |
| [batches-record.py](batches-record.py), [batches.jsonl](batches.jsonl) | `batches-record.py m2h,nowh 9 batches.jsonl 4 8`: timers fired per expiry pass, with the before and on trees instrumented by [batches-old.patch](batches-old.patch) and [batches-new.patch](batches-new.patch), and the diagnostic printing the histogram. |
| [summarize.py](summarize.py), [summary.md](summary.md) | The summary, generated from the record files. |

## Result

Numbers below are from [summary.md](summary.md).

**The readiness selector's p50 regression is not stall rescue.** In the unaligned periodic diagnostic, p50 lateness rose from 65 µs before to 90 µs with stall rescue on and 89 µs with it off (+38% and +37%). CPU fell by 9–10% in both. It comes from the rest of the change.

The instrumented builds show part of the reason:
- **4 workers:** the same expiries arrive in 23% fewer, larger passes (228,402 against 176,501 per run). Each timer then waits behind more neighbours. Cheaper registration makes tasks re-arm in tighter bursts, because each task's next deadline is set when it runs.
- **8 workers:** passes fall by only 5%, so batching explains less of the rise there (22 to 30 µs).

**The monitor's own cost is small.**
- **CPU:** in the unaligned periodic diagnostic on io_uring, where deadlines fall due continuously and the monitor wakes about 3,450 times per run, the workers used 2.70 s of CPU with it on and 2.64 s with it off. Both are below the 2.90 s before stall rescue. On the readiness selector, worker CPU with the monitor on was 1.6% lower than off in the unaligned case and 1.2% higher in the aligned case.
- **Lateness:** the unaligned io_uring p50 was 21 µs on and 17 µs off. Every build ranged over about 6.5 to 31.5 µs, and an earlier, unrecorded 11-run comparison found no difference, so that gap is not established. p99 was within 3% between on and off in every periodic case. It was better than before stall rescue in each case except the readiness selector's unaligned one, which was 3–5% higher with stall rescue on or off.
- **Pulse:** it cost 0.53 ms of CPU per run with the monitor on, 0.39 ms off and 0.42 ms before. That is about 0.1 ms for its 5 wakes per run.

**Oversubscription does not make rescues costly.**
- **8 workers beside four busy-loop processes:** rescued timers rose to 0.31–0.54% of expiries. p50 changed by 1–4% and CPU by about 1%. p99 lateness on the readiness selector was 14% better with stall rescue on than before it; on io_uring it was 3% higher with it on, against 11% higher with it off.
- **4 workers on two CPUs:** p99 lateness was 24% (readiness) and 40% (io_uring) better than before, with or without the monitor, from the timer queue changes.
- **8 workers on an idle host:** the readiness selector's p50 rose with stall rescue on and off alike, as in the 4-worker case.

## Not recorded

While looking for the monitor's CPU cost on io_uring, I compared variants of the monitor whose output was printed but not recorded:
- a monitor that never marks;
- one that skips the locked recheck of published deadlines;
- one that never makes a pass;
- one that only wakes on a timer.

The last two left the monitor's plan unset, so every published deadline unparked it, tens of thousands of times per run. Those variants only showed that a worker unparking the monitor is expensive. The others were too noisy to attribute the cost. No conclusion is drawn from them.

## Limits

- One host. Lateness medians in the periodic diagnostic vary widely between runs: p50 ranges overlap in every unaligned case.
- Worker sleeps (voluntary context switches) varied between about 35,000 and 200,000 per run within a single build, so they are not compared.
- The rescue counts in the oversubscription runs use the labels from before decision 16.
