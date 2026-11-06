# Timer lateness after the timer queue rework

These results look for the cause of the readiness selector's higher p50 lateness in the unaligned periodic diagnostic (1,024 tasks re-arming 1 ms timers) after the timer queue rework, which [rescue-costs](../rescue-costs/readme.md) traced to the rest of stall rescue's change rather than the monitor. They then measure one change, leaving the idle wakes of an owner's expiry pass to the ready batch that runs the tasks it woke. That change was not adopted: it helps the unaligned diagnostic but raises steady-load p99 at 4 workers and io_uring CPU in the unaligned diagnostic (see [Deferring an expiry pass's idle wakes](#deferring-an-expiry-passs-idle-wakes)).

Hana: Intel Xeon E3-1240 v6 (4 cores, 8 threads), `powersave` governor, TSC clock source, idle states down to C8 (exit latency 200 µs), release builds, unpinned. Every run held `/tmp/base-scheduler-perf.lock`. Builds:

- **before:** `main` at `7b158df`, before stall rescue;
- **main:** `140cc25`;
- **copy:** `140cc25` copied to another path, a control for code layout, since Cargo hashes the package path into the build;
- **c1:** `140cc25` with the deferral in [deferral.patch](deferral.patch), which also applies to `cd51eee`. It is not merged.

The diagnostic is `timer-diagnostic.rs` from the [stall rescue scripts](../../../echo/results/stall-rescue/scripts/), built as its own package against each tree from [diag.Cargo.toml](../rescue-costs/harness/diag.Cargo.toml). [scripts/run.py](scripts/run.py) runs every build in random order within each round, 9 rounds; a build named `name=BINARY@WORK@spread` runs with `CORE_TIMER_WORK=WORK` and, with `spread`, `CORE_TIMER_SPREAD=1`. [scripts/pairs.py](scripts/pairs.py) and [scripts/show.py](scripts/show.py) print the tables. Changes are the median of per-round changes, with the number of rounds lower than the base in brackets. The `quick` directory holds the records of the shared `quick.py` comparisons, whose layout control is a copy of the base tree at another path.

## The regression is mostly the diagnostic's feedback

**The rework made each expiry cheaper, and in this diagnostic cheaper expiries raise p50.** Each task's next deadline is the time it ran plus 1 ms, so deadlines that fire in one pass re-arm within that pass's run time, and clusters of deadlines merge and do not split. The cheaper each task's turn, the tighter a cluster stays, so each timer waits behind more neighbours. With the deep idle states, a worker that sleeps between clusters also wakes late.

The [work sweep](work-sweep.jsonl) runs one build, main, with `CORE_TIMER_WORK` iterations of a multiply-add loop after each expiry. Only the task's cost changes:

| Work | p50 µs, 4 workers | p99 µs | CPU s | Passes that fired |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 74.7 (51.3–91.5) | 248 | 9.30 | 237,000 |
| 50 | 54.3 (41.1–68.6) | 225 | 9.58 | 296,000 |
| 100 | 49.0 (37.3–57.5) | 203 | 9.44 | 364,000 |
| 200 | 31.9 (25.2–46.8) | 180 | 9.74 | 439,000 |
| 400 | 24.8 (7.8–38.3) | 340 | 9.39 | 576,000 |
| before, 0 | 56.5 (47.1–71.9) | 224 | 10.13 | 314,000 |

Adding 50 iterations per task gave main the p50 of before. CPU stayed within 5%, and passes rose: the same expiries arrived in more, smaller passes. At 8 workers p50 fell from 19.4 µs at no work to 8.0 µs at 400, but CPU rose there too, by 17% at 50 iterations and 101% at 400. The sweep shows that per-task cost moves this diagnostic's p50; it does not measure how much cheaper the rework made each expiry, so it supports the explanation rather than proving that it accounts for the whole difference. Pass counts come from the crate probe in [scripts/probe.rs](scripts/probe.rs), applied by [scripts/instrument.py](scripts/instrument.py) and printed by [scripts/diag-count.rs](scripts/diag-count.rs). The probe itself changes p50 somewhat: in [crate-probe-and-order.jsonl](crate-probe-and-order.jsonl), before was 74.4 and main 85.5 µs with it, against 68.7 and 89.4 µs without, so it narrowed the 4-worker gap from about 31% to 14%. The sweep's trend is within one probed build, so this does not affect it, but its comparison with before is approximate.

The diagnostic is very sensitive to per-task cost. A harness that recorded more per task ([scripts/diag-probe.rs](scripts/diag-probe.rs), [probe-harness.jsonl](probe-harness.jsonl)) brought both builds to about 40 µs at 4 workers, and the difference between them disappeared.

**At 4 workers, pass order and batch size are not distinguishable from build-to-build noise; at 8 workers the old pass recovered the gap.** [variants-and-spread.jsonl](variants-and-spread.jsonl) compares main with expiry variants from [scripts/variant.py](scripts/variant.py):

| Variant against main | p50, 4 workers | p99 | CPU | p50, 8 workers |
| --- | ---: | ---: | ---: | ---: |
| before | −15.7% (9/9) | −6.9% | +6.6% | −33.9% (8/9) |
| `seq`: deadline order under one lock | −7.1% (8/9) | +1.3% | +4.1% | −26.1% (6/9) |
| `inter`: the old pass, one pop and wake at a time | −4.7% (6/9) | +6.1% | +6.8% | −34.0% (7/9) |
| `batch32`: 32 wakers per lock hold | −12.1% (9/9) | +12.5% | +1.8% | −28.5% (8/9) |

In this run each variant lowers p50 and raises CPU, as the work sweep does. [layout.jsonl](layout.jsonl) found that identical source at another path changes this diagnostic's p50 by +12.6% at 4 workers (4 of 9 rounds lower; one copy run had a p50 of 18 µs), so 4-worker differences of this size between separately built trees are not attributed to the variants, although `batch32` was lower in 9 of 9 rounds. At 8 workers, `inter`, the old pass, matched before in this run (−34.0% against −33.9%) and recovered more than the gap in [crate-probe-and-order.jsonl](crate-probe-and-order.jsonl) (−27.9%, 9/9, against before's −16.7%), so the old pass may account for part of the 8-worker difference. That run also measured `seq` and `inter` at 4 workers (`inter` −0.5%, 5/9) and found `seq` at 8 workers with p50 +7.4% and CPU −28%, so the variants do not all trade p50 for CPU in the same way.

**At 8 workers, low lateness goes with many worker sleeps.** Across main's runs, p50 tracks worker sleeps closely (Spearman's ρ −0.80 in [work-sweep.jsonl](work-sweep.jsonl) and −0.92 in [probe-harness.jsonl](probe-harness.jsonl)) and fired passes less closely (−0.40 and −0.62): about 10 µs with about 400,000 sleeps, and 27 µs in the one run with about 100,000, at CPU times from 4.8 to 18 s ([probe-harness.jsonl](probe-harness.jsonl)). The inference, not measured directly, is that p50 depends on how far deadlines have merged in a run, and that this state drives the run-to-run bimodality. [rescue-costs](../rescue-costs/readme.md) found that 8-worker pass counts did not separate the builds; that is consistent, since here too passes track p50 only loosely.

## A diagnostic without the feedback

[scripts/diag-spread.rs](scripts/diag-spread.rs) adds `CORE_TIMER_SPREAD=1`, which gives task *i* of *n* a fixed phase of *i*/*n* periods. Its deadlines are `epoch + phase + k × delay`, so they do not depend on when it ran, and they stay evenly spread. Lateness then measures the scheduler's response at a steady million expiries per second. From [layout.jsonl](layout.jsonl), readiness selector:

| Workers | Build | p50 µs | p99 µs | CPU s |
| --- | --- | ---: | ---: | ---: |
| 4 | before | 3.103 (3.08–3.16) | 8.68 | 14.22 |
| 4 | main | 3.243 (3.19–3.33) | 10.02 | 14.00 |
| 4 | copy | 3.205 (3.19–3.26) | 10.08 | 13.97 |
| 8 | before | 4.415 (4.39–4.49) | 57.4 | 28.12 |
| 8 | main | 4.691 (4.67–4.71) | 44.1 | 27.34 |
| 8 | copy | 4.683 (4.67–4.74) | 45.4 | 27.22 |

The layout control changed p50 by −0.1% and +0.1% here, against +12.6% and +1.8% in the default mode; in the other spread-mode runs it stayed within 1%. Main's p50 is higher than before by 4.5% at 4 workers and 5.8% at 8 workers (no round lower), with 1.5–3% less CPU; [deferral-spread-iouring.jsonl](deferral-spread-iouring.jsonl) found 3.0% and 6.1% on io_uring. That is p50 lateness 0.14 to 0.28 µs later, which this mode resolves and the default mode cannot. The spread mode keeps workers busier (14 s of CPU at 4 workers against 9 s), so it does not replace the default mode's sparse wakes.

## Deferring an expiry pass's idle wakes

Every task woken by the owner's expiry pass was pushed to the owner's own queue and then called `wake_idle`, which wakes a sleeping worker if there is one. A pass of *n* timers could wake every idle neighbour, and a pass of one timer put an interrupt of a neighbour (an eventfd write and an inter-processor wake) ahead of the task it was due to run. In the [probe harness runs](probe-harness.jsonl), only 2% (4 workers) to 9% (8 workers) of expiries ran on a thread other than the one that registered them. The owner does not need these wakes for progress: it is running, and it searches its own queue before it can advertise sleep. `run_ready_batch` already wakes one idle worker when the batch it starts holds more than one task. The change suppresses the per-task wake only inside the owner's own pass; rescues, remote pushes and every other wake are unchanged.

Default diagnostic, from `quick.py` (candidate change, then the layout control's change):

| Workers | Backend | p50 | p99 | CPU | Record |
| --- | --- | --- | --- | --- | --- |
| 4 | native | −20.3% (9/9); layout +7.2% | −4.9%; −0.5% | +1.9%; +3.0% | [quick](quick/timers-timer-batching-1-20261106-215125.jsonl) |
| 8 | native | −44.0% (9/9); +6.4% | −50.3%; +1.2% | −14.7%; −2.6% | same |
| 4 | iouring | −43.6% (7/9); −8.1% | −3.7%; +0.2% | +4.2%; +0.6% | same |
| 8 | iouring | −21.5% (8/9); −4.3% | −86.5%; +1.1% | +19.1%; +2.3% | same |
| 4 | iouring | −45.3% (7/9); +19.4% | −4.1%; −0.6% | +7.9%; −6.3% | [quick](quick/timers-timer-batching-1-20261106-222817.jsonl) |
| 8 | iouring | −19.5% (7/9); −3.5% | −21.8%; +0.1% | +31.3%; +5.7% | same |

[deferral.jsonl](deferral.jsonl), a separate run on the readiness selector, found −13.9% (7/9) at 4 workers and −28.5% (8/9) at 8, with the copy at +1.9% and −1.9%. Before was −23.7% from main in the same run, and the first [quick](quick/timers-main-20261106-200604.jsonl) run put main 34% above before at 4 workers. The change recovers about 60–80% of the 4-worker regression, and more than all of it at 8 workers. On io_uring, CPU rose in both runs, by 4–8% at 4 workers and 19–31% at 8, which gives back the rework's CPU reduction on that backend in this diagnostic.

Spread diagnostic, change from main ([deferral-spread-native.jsonl](deferral-spread-native.jsonl), [deferral-spread-iouring.jsonl](deferral-spread-iouring.jsonl); the readiness selector was also measured in [deferral.jsonl](deferral.jsonl) with the same direction everywhere):

| Workers | Backend | p50 | p99 | CPU |
| --- | --- | ---: | ---: | ---: |
| 4 | native | +4.7% (0/9) | +49.5% (1/9) | −14.5% (9/9) |
| 8 | native | −1.5% (9/9) | −60.7% (9/9) | −30.3% (9/9) |
| 4 | iouring | −3.0% (8/9) | +15.5% (0/9) | −19.6% (9/9) |
| 8 | iouring | +0.2% (3/9) | −55.5% (9/9) | −40.7% (9/9) |

At a steady load the change removes 14–41% of CPU. At 4 workers on the readiness selector it raises p50 by 0.14 µs and p99 from about 11 to 16 µs. The likely reason, not measured, is that neighbours woken by the old wakes were awake or in shallow idle states when their own timers fell due.

**Not adopted.** For: the unaligned diagnostic's p50 falls by 14–45%, steady-load CPU by 14–41%, and 8-worker p99 falls in both modes (−50% to −87% in the unaligned diagnostic, −55% to −61% at a steady load). Against: at a steady load on 4 workers p99 rises (+49.5% on the readiness selector, +15.5% on io_uring), and io_uring CPU rises in the unaligned diagnostic (19–31% at 8 workers; at 4 workers +7.9% in one run and +4.2%, higher in only 5 of 9 rounds, in the other). The unaligned diagnostic's p50 regression is mostly that diagnostic's own feedback, and neither mode stands for real applications, so the evidence does not clearly favour the change. A narrower version, keeping the idle wake for a single-timer pass that came straight out of an idle wake, was not tried.

TCP echo with borrowed buffers on the readiness selector, 4 workers, was unchanged ([quick](quick/echo-timer-batching-1-20261106-220354.jsonl)): throughput +0.6% and +0.8% at 256 and 32 connections, against −0.3% and +1.0% for the layout control.

## Limits

- One host, with deep idle states. Lateness in both modes depends on wake latency from idle, which differs between hosts.
- CPU at 8 workers in the default mode varies from 4.5 to 18 s within one build, so its changes there are not attributed.
- The work sweep varies the task's cost, not the timer operations' cost, and the old build's per-expiry costs (a shared reference count per registration, command queue checks) were not measured one by one. A build that restored the shared reference count (`mainarc` in [probe-harness.jsonl](probe-harness.jsonl)) was measured only with the heavier probe harness, where neither build regressed.
- The spread mode is in this record's script ([scripts/diag-spread.rs](scripts/diag-spread.rs)), not in the timer benchmark package; the hand-built harness sources here should become a `periodic` mode of `benchmarks/timers` in a follow-up.
- Rows do not record `CORE_TIMER_WORK`, `CORE_TIMER_SPREAD` or the binary; only the build name encodes them. `run.py` was run under `flock /tmp/base-scheduler-perf.lock`, which it does not take itself. Runs used 9 rounds.
- The first quick record ([timers-main-20261106-200604.jsonl](quick/timers-main-20261106-200604.jsonl), main against before) predates `quick.py`'s layout control and has none. The quick records compare `trees/main` (`140cc25`) with `trees/timers-before` (`7b158df`) or `trees/timer-batching-1` (`140cc25` with [deferral.patch](deferral.patch)); `quick.py` itself is not in this repository.
