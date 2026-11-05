## Stuck marks by worker phase

Both periodic runs, 24 runs each: `phases.jsonl` (marks only) and `wakes.jsonl` (marks and wake latencies).

| Phase when marked | Marks | Labelled stuck poll | Labelled long batch |
| --- | --- | --- | --- |
| idle | 26 | 17 | 9 |
| tasks | 27 | 11 | 16 |
| driver | 1 | 1 | 0 |
| search | 3 | 3 | 0 |
| timers | 6 | 5 | 1 |
| all | 63 | 37 | 26 |

Of the marks in the tasks or driver phase, 19 had entered it after the monitor read the clock (time in phase 0), and 8 had been running tasks for more than 1 ms.

A mark lasting 40.0 ms: the worker ran for 1.2% of it, with 2 involuntary switches.
A mark lasting 19.4 ms: the worker ran for 19.4% of it, with 60 involuntary switches.
A mark lasting 19.5 ms: the worker ran for 0.4% of it, with 0 involuntary switches.
A mark lasting 12.2 ms: the worker ran for 2.2% of it, with 1 involuntary switches.
A mark lasting 60.3 ms: the worker ran for 1.1% of it, with 3 involuntary switches.

## Slow wakes in the periodic workload

From `wakes.jsonl`. A slow wake took more than 250 µs from another thread's wake to the worker leaving idle. The missed-wake signature is a wake more than 200 µs before the worker's own idle timeout, followed by a resume within 100 µs of that timeout.

| Backend, deadlines | Wakes | Slow | Missed-wake signature | Late after both |
| --- | --- | --- | --- | --- |
| iouring, aligned | 19,705 | 145 (0.736%) | 8 | 26 |
| iouring, unaligned | 437,560 | 198 (0.045%) | 46 | 36 |
| native, aligned | 29,403 | 103 (0.350%) | 6 | 15 |
| native, unaligned | 186,789 | 49 (0.026%) | 0 | 6 |
| all | 673,457 | 495 (0.074%) | 60 | 83 |

## Lost-wake test

From `wake-latency.jsonl`: `wake_latency BACKEND WORKERS 20000`, a task woken from another thread on an idle worker with no timers pending, so a missed wake would wait indefinitely.

| Host | Backend | Workers | Wakes | Lost (over 2 s) | Over 10 ms | Over 1 ms | p50 µs | p99 µs | Max µs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| idle | native | 1 | 20,000 | 0 | 0 | 3 | 74.6 | 108.0 | 3119 |
| idle | iouring | 1 | 20,000 | 0 | 0 | 0 | 30.3 | 146.3 | 228 |
| idle | native | 4 | 20,000 | 0 | 0 | 0 | 79.6 | 108.4 | 241 |
| idle | iouring | 4 | 20,000 | 0 | 0 | 0 | 30.1 | 145.3 | 239 |
| loaded | native | 1 | 20,000 | 0 | 0 | 0 | 6.9 | 32.1 | 632 |
| loaded | iouring | 1 | 20,000 | 0 | 0 | 1 | 6.9 | 31.9 | 3064 |
| loaded | native | 4 | 20,000 | 0 | 0 | 0 | 7.3 | 32.1 | 196 |
| loaded | iouring | 4 | 20,000 | 0 | 0 | 1 | 7.3 | 32.0 | 1023 |
