# Stall rescue acceptance measurements

These results check stage 2 of [stall rescue](../../../../design/stall-rescue.md) (timers) against its performance budget. The baseline is `main` at `7b158df`; the branch is `7adde39`, whose only later change is test code.

Hana: Linux 7.2.14, Intel Xeon E3-1240 v6 (4 cores, 8 threads), `powersave` governor, rustc 1.99.1, release builds. Nothing else ran on the host; every sample took `/tmp/base-scheduler-perf.lock`.

| File | Contents |
| --- | --- |
| [acceptance.jsonl](acceptance.jsonl) | 1,170 samples from [`scripts/budget.py`](scripts/budget.py) with 9 repeats, `main` and branch interleaved in random order within each repeat. |
| [acceptance.md](acceptance.md) | Its summary from [`scripts/summarize.py`](scripts/summarize.py): the median, minimum and maximum on `main`, the branch median, and a verdict against the budget for that row (≤ ~3% for timer cases; within `main`'s range for echo, spawn and idle cases). |
| [stall.md](stall.md) | [`scripts/quick.py`](scripts/quick.py), 9 pairs: a 10 ms root sleep behind a 200 ms stall on a 2-worker scheduler, and a 1 Hz pulse timer. |

## Result

- **Stall:** worst-case lateness falls from 185.2 ms to 1.33 ms (1.21–1.49 ms); median lateness is unchanged.
- **Improvements:** `sleep yield` and `sleep refresh` are 5–10% faster on both selectors, mostly from registrations borrowing the scheduler instead of cloning its `Arc`. Root sleeps use 27% less CPU, with 17% lower p50 lateness.
- **Echo matrix, spawn and idle:** within noise. Six server-CPU or p50 rows fall just outside `main`'s range, by 0.9–3.0%.

Over budget, for the maintainer's decision (design decision 4):

| Case | Metric | `main` | Branch | Change | Note |
| --- | --- | --- | --- | --- | --- |
| diag native 4/1024/1 ms, unaligned | late p50 | 64.0 µs | 80.5 µs | +26% | CPU −12%. Aligned deadlines: p50 +3%, p99 −5%. See the design's implementation notes. |
| diag native 4/1024/1 ms, unaligned | late p99 | 234.8 µs | 242.7 µs | +3.4% | |
| `pulse` (1 Hz timer) | CPU | 1.05 ms | 1.35 ms | +28% | The monitor wakes once per deadline, *S* after it; inherent to deadline-driven detection (decision 15). |
| timers `cycle` 64/4 (both selectors) | late p50 | 2.4–5.0 µs | 2.8–5.7 µs | +9 to +16% | Sub-microsecond; `main`'s own range is wider than the change. |
| timers native reuse `cycle` 64/4 | late p99 | 37 µs | 85 µs | +128% | Bimodal: `main` ranges 9–129 µs. |
| timers `cycle` 64/4 | CPU per op | — | — | +7 to +10% | Ranges span about 2×; the same cases were −10 to −12% in other rows and runs. |
| diag 1/1/10 µs | late p50 / p99 | 5.2 / 14.9 µs | 5.5 / 15.7 µs | +5 to +6% | |
| diag iouring 4/1024/1 ms | late p50 | 20.8 µs | 21.6 µs | +3.5% | Range 6.8–34 µs. |
