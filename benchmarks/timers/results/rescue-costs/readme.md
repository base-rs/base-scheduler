# Stall rescue costs

These results separate what [stall rescue](../../../../design/stall-rescue.md)'s monitor costs from the rest of its change, and check whether rescues cost more when workers compete for CPUs. They follow the [acceptance run](../../../echo/results/stall-rescue/readme.md). That run's largest consistent regression was the readiness selector's p50 lateness in the 1,024-timer periodic diagnostic, and it had no build with stall rescue switched off.

Three builds were compared:

- **before:** `main` at `7b158df`, before stall rescue;
- **on:** `793cbbf`, `main` with stall rescue and before decision 16's classification, which changes only diagnostics builds;
- **off:** `793cbbf` with [off.patch](off.patch), which makes `stall_rescue` default to off. It keeps the rest of the change, including the lockable timer queues and published deadlines, but starts no monitor.

The periodic diagnostic and pulse are the stall rescue scripts from the acceptance run, `timer-diagnostic.rs` and `rb.rs` in [`stall-rescue/scripts`](../../../echo/results/stall-rescue/scripts/). Each was built as its own package against each tree, from [harness/diag.Cargo.toml](harness/diag.Cargo.toml) and [harness/rb.Cargo.toml](harness/rb.Cargo.toml) with `TREE` replaced by the tree's path, with the lockfiles in [harness](harness/): `diag-before.Cargo.lock` and `rb-before.Cargo.lock` for the before tree, whose native crate also depends on `crossbeam-queue` and `futures-channel`, and `diag.Cargo.lock` and `rb.Cargo.lock` for the on and off trees.

Hana: Linux 7.2.14, Intel Xeon E3-1240 v6 (4 cores, 8 threads), `powersave` governor, rustc 1.99.1, release builds, unpinned unless stated. Every run held `/tmp/base-scheduler-perf.lock`, and nothing else of ours ran on the host. The drivers take build names `m2` (before), `now` (on) and `nowoff` (off), and find binaries in `diag-{build}/target/release/diag` and `rb-{build}/target/release/rb`. `costs.py` and `oversub.py` look beside the script, and `batches-record.py` looks in the current directory.

```sh
# For each tree and harness: copy the script to diag-{build}/src/main.rs or rb-{build}/src/main.rs, write its Cargo.toml, and build.
(cd diag-$build && cargo build --release)
flock /tmp/base-scheduler-perf.lock python3 costs.py m2,now,nowoff 15 costs.jsonl
flock /tmp/base-scheduler-perf.lock python3 oversub.py m2,now,nowoff 9 COUNTS_BINARY oversub.jsonl
flock /tmp/base-scheduler-perf.lock python3 batches-record.py m2h,nowh 9 batches.jsonl 4 8
python3 summarize.py . > summary.md
```

| File | Contents |
| --- | --- |
| [costs.py](costs.py), [costs.jsonl](costs.jsonl) | The periodic diagnostic on both selectors, aligned and unaligned, and pulse, with each run's per-thread CPU time and context switches; 15 repeats, the three builds back to back in random order. |
| [oversub.py](oversub.py), [oversub.jsonl](oversub.jsonl) | The periodic diagnostic with 8 workers, with 8 workers beside four busy-loop processes, and with 4 workers pinned to CPUs 0 and 1; 9 repeats. Its rescue counts come from `COUNTS_BINARY`, the timer benchmark's `periodic` mode built with `diagnostics` at `f59cba4`, the revision [rescue-counts](../rescue-counts/readme.md) used: scheduler code from `0b4c540`, without `fc987a6`'s monitor startup fix and before decision 16's labels. |
| [batches-record.py](batches-record.py), [batches.jsonl](batches.jsonl) | Timers fired per expiry pass on the readiness selector. The before and on trees are instrumented by [batches-old.patch](batches-old.patch) and [batches-new.patch](batches-new.patch), and the diagnostic prints the histogram with [harness/diag-batches.patch](harness/diag-batches.patch). The builds are named `m2h` and `nowh`. |
| [summarize.py](summarize.py), [summary.md](summary.md) | The summary, generated from the record files. |

## Result

Numbers below are from [summary.md](summary.md).

**The readiness selector's p50 regression is not the monitor.** In the unaligned periodic diagnostic, paired by repeat, p50 lateness was higher than before stall rescue in 14 of 15 repeats with stall rescue on, and in 14 of 15 with it off. The median per-repeat change was +19.5% on and +25.8% off. Process CPU fell by 9–10% in both. The regression comes from the rest of the change: the lockable timer queues, published deadlines and the worker loop's timer pass.

The instrumented builds suggest one mechanism. At 4 workers, the same expiries arrived in fewer, larger passes: 176,501 per run against 228,402 (ranges 143,622–199,877 and 188,244–267,726). Each timer then waits behind more neighbours. One explanation is that cheaper registration makes tasks re-arm in tighter bursts, since each task's next deadline is set when it runs, but other changes to the timer pass could also change how often a pass finds timers due. Neither was isolated, and the off build was not instrumented. At 8 workers, pass counts overlapped completely (223,606–704,034 against 397,355–659,018), so batching does not explain the rise there.

**The monitor's own cost is a few percent of CPU on io_uring, and not consistent on the readiness selector.** Paired by repeat, with stall rescue on against off:
- **io_uring, unaligned:** process CPU +5.0% and worker CPU +4.1%, with on higher in 12 of 15 repeats. This is the case where deadlines fall due continuously and the monitor wakes about 3,450 times per run. Total CPU was still below before stall rescue.
- **io_uring, aligned:** process CPU +2.7% and worker CPU +2.9% (11 and 10 of 15).
- **Readiness selector:** from −1% to +2%, with on higher in 6 to 9 of 15 repeats.
- **The monitor thread itself:** about 0.01 s of CPU per run on io_uring and 0.05 s on the readiness selector, at 10 ms resolution.
- **Pulse:** process CPU was 0.53 ms per run on, 0.39 ms off and 0.42 ms before, about 0.1 ms for the monitor's 5 wakes per run.
- **Lateness:** p50 was not consistently higher with the monitor on. The median per-repeat change was −7.0% on io_uring unaligned (on higher in 7 of 15), −10.5% on io_uring aligned (3 of 15), and +1 to +3% on the readiness selector (8 and 10 of 15). p99 was within 3% between on and off in every periodic case.

**Oversubscription did not make rescues costly.**
- **8 workers beside four busy-loop processes:** rescued timers rose to 0.31–0.54% of expiries. p50 changed by 1–4% and CPU by about 1%. p99 ranges overlapped widely, for example 413–772 µs before, 429–617 µs on and 307–757 µs off on the readiness selector, so no p99 difference is claimed.
- **4 workers pinned to two CPUs:** p99 lateness was 24% (readiness) and 40% (io_uring) lower than before stall rescue, with or without the monitor, and the ranges did not overlap. This comes from the rest of the change.
- **8 workers on an idle host:** the readiness selector's p50 rose with stall rescue on and off alike, as in the 4-worker case.

The design's earlier note that rescue off reduced the unaligned regression to about 9% came from one run of 11 pairs on an earlier build. These paired results supersede it.

## Not measured

- The off build was compared only on the periodic diagnostic and pulse. The echo matrix, spawning and idle CPU, which had the largest median shifts in the acceptance run, were not compared with stall rescue off.
- The oversubscription runs are not the design's fixed-rate p99 with every core occupied, nor its workload with occasional undeclared blocking; both remain unmeasured.

## Not recorded

While looking for the monitor's CPU cost on io_uring, I ran variants of the monitor whose output was printed but not recorded:
- one that never marks;
- one that skips the locked recheck of published deadlines;
- one that never makes a pass;
- one that only wakes on a timer.

No conclusion is drawn from them.

## Limits

- **One host.** Lateness medians in the periodic diagnostic vary widely between runs, and p50 ranges overlap in every unaligned case. The paired comparisons above are more reliable than the medians in the tables.
- **Per-thread CPU** comes from `/proc` at 10 ms resolution, sampled every 50 ms, so the last sample can miss up to 50 ms per busy thread. It covers each thread's whole life, while the diagnostic's process CPU is measured from its start.
- **Worker sleeps** (voluntary context switches) varied between about 35,000 and 200,000 per run within one build on io_uring, so they are not compared.
- **Rescue shares** are upper bounds: rescues are counted over the whole run, and expiries only inside the measurement window. Their labels come from before decision 16.
- **The batch histograms** count only passes that fired a timer, and the on build's excludes rescue passes.
- **A pattern not claimed:** on io_uring aligned, p50 was lower with stall rescue on than off in 12 of 15 repeats. No mechanism was looked for.
