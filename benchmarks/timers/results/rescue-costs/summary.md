## Periodic diagnostic and pulse

From `costs.jsonl`: 15 repeats, builds in random order within each. Worker CPU and sleeps (voluntary context switches) are summed over the worker threads; monitor wakes are the monitor thread's voluntary context switches.

| Case | Metric | before | on | off | on vs before | off vs before |
| --- | --- | --- | --- | --- | --- | --- |
| periodic iouring 4/1024/1000us | p50 lateness µs | 15.76 (12.7–23.7) | 20.83 (6.58–27.1) | 16.91 (6.54–31.5) | +32.2% | +7.3% |
| periodic iouring 4/1024/1000us | p99 lateness µs | 249.1 (232–252) | 239.4 (225–244) | 239.5 (228–244) | -3.9% | -3.9% |
| periodic iouring 4/1024/1000us | process CPU s | 2.976 (2.8–3.27) | 2.823 (2.59–3.44) | 2.736 (2.52–3.48) | -5.1% | -8.1% |
| periodic iouring 4/1024/1000us | worker CPU s | 2.9 (2.73–3.18) | 2.7 (2.48–3.32) | 2.64 (2.43–3.39) | -6.9% | -9.0% |
| periodic iouring 4/1024/1000us | worker sleeps | 5.392e+04 (3.53e+04–1.11e+05) | 6.594e+04 (3.59e+04–1.87e+05) | 5.251e+04 (3.48e+04–1.98e+05) | +22.3% | -2.6% |
| periodic iouring 4/1024/1000us | monitor wakes | 0 (0–0) | 3447 (3.42e+03–3.54e+03) | 0 (0–0) | n/a | n/a |
| periodic iouring 4/1024/1000us aligned | p50 lateness µs | 254.5 (109–269) | 210.7 (102–261) | 245.2 (101–259) | -17.2% | -3.6% |
| periodic iouring 4/1024/1000us aligned | p99 lateness µs | 357.9 (274–372) | 329.7 (195–359) | 321.8 (183–357) | -7.9% | -10.1% |
| periodic iouring 4/1024/1000us aligned | process CPU s | 2.897 (2.79–3.26) | 2.681 (2.55–2.95) | 2.624 (2.47–2.78) | -7.4% | -9.4% |
| periodic iouring 4/1024/1000us aligned | worker CPU s | 2.83 (2.7–3.16) | 2.58 (2.47–2.85) | 2.53 (2.4–2.69) | -8.8% | -10.6% |
| periodic iouring 4/1024/1000us aligned | worker sleeps | 1.924e+04 (1.9e+04–1.95e+04) | 1.91e+04 (1.88e+04–1.96e+04) | 1.905e+04 (1.89e+04–1.93e+04) | -0.8% | -1.0% |
| periodic iouring 4/1024/1000us aligned | monitor wakes | 0 (0–0) | 2897 (2.63e+03–2.99e+03) | 0 (0–0) | n/a | n/a |
| periodic native 4/1024/1000us | p50 lateness µs | 65.06 (51.5–86) | 89.66 (52–98.2) | 89.22 (61.4–99.2) | +37.8% | +37.1% |
| periodic native 4/1024/1000us | p99 lateness µs | 229.6 (216–238) | 240.9 (233–293) | 237.1 (227–270) | +4.9% | +3.2% |
| periodic native 4/1024/1000us | process CPU s | 9.906 (8.39–10.7) | 8.911 (7.96–9.76) | 9.003 (7.99–9.21) | -10.0% | -9.1% |
| periodic native 4/1024/1000us | worker CPU s | 9.76 (8.27–10.5) | 8.7 (7.74–9.55) | 8.84 (7.85–9.05) | -10.9% | -9.4% |
| periodic native 4/1024/1000us | worker sleeps | 3.836e+04 (2.36e+04–5.24e+04) | 4.31e+04 (2.6e+04–4.65e+04) | 3.333e+04 (2.26e+04–5.81e+04) | +12.4% | -13.1% |
| periodic native 4/1024/1000us | monitor wakes | 0 (0–0) | 3621 (3.6e+03–3.65e+03) | 0 (0–0) | n/a | n/a |
| periodic native 4/1024/1000us aligned | p50 lateness µs | 449.5 (315–496) | 454.4 (259–489) | 447.5 (354–485) | +1.1% | -0.5% |
| periodic native 4/1024/1000us aligned | p99 lateness µs | 779.3 (684–808) | 729.1 (655–776) | 728.1 (630–765) | -6.4% | -6.6% |
| periodic native 4/1024/1000us aligned | process CPU s | 10.49 (8.15–11.1) | 9.717 (5.94–10.3) | 9.594 (8.7–10.2) | -7.4% | -8.5% |
| periodic native 4/1024/1000us aligned | worker CPU s | 10.34 (8.02–11) | 9.55 (5.81–10.1) | 9.44 (8.62–10.1) | -7.6% | -8.7% |
| periodic native 4/1024/1000us aligned | worker sleeps | 1.899e+04 (1.88e+04–1.93e+04) | 1.896e+04 (1.86e+04–1.92e+04) | 1.901e+04 (1.85e+04–1.91e+04) | -0.2% | +0.1% |
| periodic native 4/1024/1000us aligned | monitor wakes | 0 (0–0) | 3224 (3.02e+03–3.33e+03) | 0 (0–0) | n/a | n/a |

## Pulse

A task woken by a timer once a second, 3 times per run.

| Case | Metric | before | on | off | on vs before | off vs before |
| --- | --- | --- | --- | --- | --- | --- |
| pulse 1 Hz | process CPU ms | 0.42 (0.367–0.467) | 0.529 (0.363–0.573) | 0.385 (0.322–0.474) | +26.0% | -8.3% |
| pulse 1 Hz | monitor wakes | 0 (0–0) | 5 (4–7) | 0 (0–0) | n/a | n/a |

## Oversubscription

From `oversub.jsonl`: the periodic diagnostic (1,024 tasks, 1 ms) with 8 workers on an idle host, 8 workers beside four busy-loop processes, and 4 workers pinned to CPUs 0 and 1; 9 repeats.

| Case | Metric | before | on | off | on vs before | off vs before |
| --- | --- | --- | --- | --- | --- | --- |
| 4 workers on 2 CPUs, iouring | p50 lateness µs | 7.791 (7.73–8.18) | 7.679 (7.63–7.86) | 7.655 (7.54–8.29) | -1.4% | -1.7% |
| 4 workers on 2 CPUs, iouring | p99 lateness µs | 182 (170–196) | 108.9 (105–118) | 103.7 (99.1–140) | -40.2% | -43.0% |
| 4 workers on 2 CPUs, iouring | process CPU s | 7.316 (7.28–7.33) | 7.131 (7.11–7.15) | 7.128 (7.08–7.17) | -2.5% | -2.6% |
| 4 workers on 2 CPUs, native | p50 lateness µs | 8.399 (8.21–8.71) | 8.359 (8.28–8.69) | 8.367 (8.23–8.62) | -0.5% | -0.4% |
| 4 workers on 2 CPUs, native | p99 lateness µs | 255.2 (241–271) | 194.6 (190–206) | 195.5 (173–205) | -23.8% | -23.4% |
| 4 workers on 2 CPUs, native | process CPU s | 7.436 (7.41–7.45) | 7.304 (7.18–7.32) | 7.308 (7.26–7.32) | -1.8% | -1.7% |
| 8 workers + 4 spinners, iouring | p50 lateness µs | 9.327 (8.92–9.75) | 9.439 (9.05–10.5) | 9.415 (8.94–10.1) | +1.2% | +0.9% |
| 8 workers + 4 spinners, iouring | p99 lateness µs | 390.7 (337–792) | 402.7 (317–643) | 432.4 (292–819) | +3.1% | +10.7% |
| 8 workers + 4 spinners, iouring | process CPU s | 14.28 (13.8–14.6) | 14.16 (13.4–14.5) | 14.11 (13.4–14.4) | -0.9% | -1.2% |
| 8 workers + 4 spinners, native | p50 lateness µs | 9.655 (9.37–10.6) | 10.04 (9.52–10.6) | 9.951 (9.32–11.4) | +4.0% | +3.1% |
| 8 workers + 4 spinners, native | p99 lateness µs | 625.7 (413–772) | 535 (429–617) | 488.2 (307–757) | -14.5% | -22.0% |
| 8 workers + 4 spinners, native | process CPU s | 14.5 (14.1–14.8) | 14.33 (13.8–14.7) | 14.36 (14.1–15.1) | -1.2% | -1.0% |
| 8 workers, idle, iouring | p50 lateness µs | 9.343 (8.27–12.6) | 9.079 (8.14–10.8) | 11.6 (7.94–18.7) | -2.8% | +24.1% |
| 8 workers, idle, iouring | p99 lateness µs | 216.8 (212–223) | 229.8 (194–240) | 221.1 (215–232) | +6.0% | +1.9% |
| 8 workers, idle, iouring | process CPU s | 6.07 (5.24–6.69) | 5.481 (5.07–6.67) | 4.857 (4.12–5.91) | -9.7% | -20.0% |
| 8 workers, idle, native | p50 lateness µs | 15.52 (9.71–24) | 23.1 (11.1–33.8) | 21.14 (9.54–24.5) | +48.9% | +36.2% |
| 8 workers, idle, native | p99 lateness µs | 223.5 (209–313) | 249.7 (230–315) | 221.2 (182–424) | +11.7% | -1.0% |
| 8 workers, idle, native | process CPU s | 7.105 (5.45–17.1) | 5.54 (4.77–16) | 8.65 (4.39–17) | -22.0% | +21.7% |

Rescues in a `diagnostics` build, 3 runs per condition (stuck polls / long batches / timers rescued, and rescued timers as a share of expiries; labels from the classification before decision 16):

| Condition | Runs |
| --- | --- |
| 4 workers on 2 CPUs, iouring | 0 / 0 / 0 (0.000%); 0 / 0 / 0 (0.000%); 0 / 0 / 0 (0.000%) |
| 4 workers on 2 CPUs, native | 0 / 0 / 0 (0.000%); 0 / 0 / 0 (0.000%); 1 / 0 / 265 (0.009%) |
| 8 workers + 4 spinners, iouring | 241 / 15 / 15074 (0.506%); 244 / 13 / 13195 (0.442%); 162 / 26 / 14250 (0.481%) |
| 8 workers + 4 spinners, native | 306 / 37 / 15931 (0.540%); 190 / 23 / 13242 (0.447%); 172 / 6 / 9281 (0.314%) |
| 8 workers, idle, iouring | 0 / 0 / 0 (0.000%); 5 / 0 / 630 (0.021%); 0 / 1 / 138 (0.005%) |
| 8 workers, idle, native | 11 / 3 / 1431 (0.049%); 5 / 7 / 568 (0.019%); 1 / 2 / 251 (0.009%) |

## Timers fired per expiry pass

From `batches.jsonl`: the readiness selector's periodic diagnostic with builds instrumented by `batches-old.patch` (before) and `batches-new.patch` (on); 9 repeats. Bucket *b* counts expiry passes that fired from 2^(b−1) to 2^b − 1 timers; the values are means per run.

| Workers | Build | p50 lateness µs | Passes per run | 16–31 | 32–63 | 64–127 | 128–255 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 4 | before | 76.0 | 228,402 | 51,069 | 27,998 | 4,061 | 196 |
| 4 | on | 91.3 | 176,501 | 41,247 | 35,074 | 6,588 | 385 |
| 8 | before | 21.9 | 489,595 | 38,475 | 8,952 | 4,694 | 1,548 |
| 8 | on | 30.0 | 464,859 | 28,316 | 9,728 | 4,348 | 2,065 |
