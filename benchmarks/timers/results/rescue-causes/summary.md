## Stuck marks

48 runs of the periodic workload (`events.jsonl`), 92 marks. The phase is read when the monitor marks the worker, before it rescues anything.

| Phase | Marks | Labelled stuck poll | Labelled long batch |
| --- | --- | --- | --- |
| idle | 50 | 28 | 22 |
| tasks | 30 | 9 | 21 |
| search | 2 | 0 | 2 |
| timers | 10 | 3 | 7 |
| all | 92 | 40 | 52 |

### Proposed classification

| Class | Marks | Labelled stuck poll today |
| --- | --- | --- |
| woken, not yet running | 50 | 28 |
| long batch or turn | 42 | 12 |

### Marks without poll progress

Scheduler counters over each mark: CPU time (`run`), run-queue delay (`delay`), and voluntary and involuntary switches.

| Case | Phase | Marked µs | Run µs | Delay µs | Voluntary | Involuntary |
| --- | --- | --- | --- | --- | --- | --- |

### Woken workers not yet running

50 of these marks have a logged clear. Over each mark, the median worker was marked for 1963 µs, ran for 843 µs and waited in the run queue for 2202 µs. In 40 of them the run-queue delay was at least half the mark: the woken thread was waiting for a CPU.

## Rescues

Every rescue that fired a timer is logged: 196 logged against 196 counted by `RescueStatistics`.

| Rescuer | Labelled stuck poll | Labelled long batch |
| --- | --- | --- |
| monitor | 33 | 34 |
| worker | 6 | 123 |

## Wake latency

From `wake-latency.jsonl`: `wake_latency BACKEND WORKERS 20000 MODE`. `thread` is a plain thread woken with `park` and `unpark`, with no scheduler. `sleep` waits 50 to 500 µs before each wake; `spin` busy-waits 0 to 30 µs, so wakes race the worker's transition to idle. A lost wake would hang the program, so every completed run lost none.

| Host | Mode | Backend | Workers | Over 1 ms | Over 10 ms | p50 µs | p99 µs | p99.9 µs | Max µs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| idle | sleep | thread | 1 | 0 | 0 | 64.4 | 86.7 | 108 | 515 |
| idle | sleep | tokio | 1 | 1 | 0 | 73.7 | 100.4 | 118 | 2774 |
| idle | sleep | native | 1 | 0 | 0 | 84.4 | 108.9 | 125 | 332 |
| idle | sleep | iouring | 1 | 0 | 0 | 32.0 | 152.9 | 162 | 361 |
| idle | sleep | tokio | 4 | 2 | 0 | 84.7 | 109.1 | 129 | 2805 |
| idle | sleep | native | 4 | 1 | 0 | 85.1 | 108.8 | 128 | 2153 |
| idle | sleep | iouring | 4 | 0 | 0 | 31.5 | 151.9 | 163 | 528 |
| idle | spin | thread | 1 | 0 | 0 | 2.6 | 4.7 | 45 | 67 |
| idle | spin | tokio | 1 | 0 | 0 | 3.7 | 6.2 | 43 | 65 |
| idle | spin | native | 1 | 0 | 0 | 4.0 | 5.3 | 20 | 62 |
| idle | spin | iouring | 1 | 0 | 0 | 3.8 | 4.6 | 18 | 62 |
| idle | spin | tokio | 4 | 0 | 0 | 4.2 | 7.4 | 37 | 386 |
| idle | spin | native | 4 | 0 | 0 | 3.8 | 5.9 | 42 | 59 |
| idle | spin | iouring | 4 | 0 | 0 | 4.0 | 5.8 | 39 | 70 |
| loaded | sleep | thread | 1 | 1 | 0 | 4.8 | 28.5 | 55 | 2126 |
| loaded | sleep | tokio | 1 | 1 | 0 | 5.9 | 30.8 | 54 | 2827 |
| loaded | sleep | native | 1 | 1 | 0 | 6.9 | 31.8 | 54 | 2061 |
| loaded | sleep | iouring | 1 | 1 | 0 | 7.0 | 31.3 | 55 | 2463 |
| loaded | sleep | tokio | 4 | 0 | 0 | 6.8 | 31.3 | 50 | 247 |
| loaded | sleep | native | 4 | 0 | 0 | 7.0 | 31.8 | 58 | 333 |
| loaded | sleep | iouring | 4 | 0 | 0 | 7.6 | 31.6 | 57 | 225 |
| loaded | spin | thread | 1 | 0 | 0 | 3.2 | 4.4 | 9 | 53 |
| loaded | spin | tokio | 1 | 0 | 0 | 4.5 | 6.3 | 18 | 55 |
| loaded | spin | native | 1 | 0 | 0 | 5.0 | 6.3 | 20 | 66 |
| loaded | spin | iouring | 1 | 0 | 0 | 5.1 | 6.6 | 10 | 63 |
| loaded | spin | tokio | 4 | 0 | 0 | 4.9 | 6.5 | 9 | 31 |
| loaded | spin | native | 4 | 0 | 0 | 4.9 | 6.8 | 20 | 66 |
| loaded | spin | iouring | 4 | 0 | 0 | 4.7 | 7.0 | 22 | 807 |
