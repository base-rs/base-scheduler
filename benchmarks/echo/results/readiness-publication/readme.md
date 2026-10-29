# Readiness publication atomics

On echo with one worker and 32 connections, Native readiness spends about 21 locked instructions per round trip, against Tokio's 13. Each edge-triggered event publishes a direction with `fetch_add` and `fetch_or` on its readiness word, sometimes another `fetch_or` for EOF or urgent data, and then locks its notification mutex, even when nobody waits. Two reductions were measured. Both are marginal, and both were rejected.

| Variant | Patch | Change |
| --- | --- | --- |
| r1 | [single-cas-and-waiter-count.patch](single-cas-and-waiter-count.patch) | One compare-and-swap publishes the generation, readiness and sticky flags. A waiter count, changed only under the notification lock, lets publication skip the lock when nobody waits. Publication's SeqCst update pairs with a listener's SeqCst registration and recheck; closure keeps the locked path |
| r2 | [single-cas.patch](single-cas.patch) | The compare-and-swap alone |

Both apply to main `aaf337eab1d0fc99e6177a61649dfc94fe6b30f1`. The echo baseline `e0` is that revision with the turn-spacing record's [echo backend patch](../turn-interval/echo-native-backend.patch). The method and environment match the [turn-spacing comparison](../turn-interval/readme.md): Hana, server on CPU 0, user-space `perf stat` counters per request.

## Echo

Medians per round trip from two interleaved cohorts in [echo-events.txt](echo-events.txt), measured with [echo-events.sh](echo-events.sh) on `native-poll` with one worker, 32 connections and 64-byte messages. e0 and r1 ran six times each, r2 three times in the second cohort, and Tokio three times afterwards for reference:

| Variant | Locked loads | Instructions | Cycles | L1d misses | Requests/s |
| --- | ---: | ---: | ---: | ---: | ---: |
| e0 | 21.2 | 1,419 | 1,334 | 46.2 | 217.4k |
| r1 | 18.3 | 1,376 | 1,276 | 46.0 | 218.8k |
| r2 | 19.2 | 1,430 | 1,265 | 44.4 | 215.8k |
| Tokio | 13.1 | 1,459 | 998 | 23.7 | 223.0k |

r1 saves three locked instructions and about 43 instructions per round trip. r2 saves two locked instructions, but its load and compare-and-swap loop add about 10 instructions, and its throughput is below every e0 run of its cohort, although it used fewer cycles than e0 in each of them. Cycles differ by up to 190 per round trip between runs of the same build, so the cycle difference is not treated as significant.

## HTTP

Five interleaved repeats of main (A) and r1 with the base-http hello server, from [rcohort.sh](rcohort.sh) and [http-cohort.jsonl](http-cohort.jsonl). Requests per second / user instructions per request:

| Workers | Server | Connections | A | r1 | Throughput change | r1 faster in |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | supervisor | 1 | 52.4k / 8,480 | 53.4k / 8,424 | +1.8% | 5 of 5 |
| 1 | supervisor | 64 | 178.6k / 5,888 | 179.9k / 5,838 | +0.7% | 3 of 5 |
| 1 | supervisor | 256 | 176.5k / 5,872 | 176.6k / 5,818 | +0.0% | 3 of 5 |
| 1 | task per connection | 1 | 53.2k / 6,691 | 53.3k / 6,644 | +0.2% | 2 of 5 |
| 1 | task per connection | 64 | 180.9k / 5,962 | 181.6k / 5,914 | +0.4% | 3 of 5 |
| 1 | task per connection | 256 | 169.1k / 5,995 | 170.7k / 5,949 | +0.9% | 3 of 5 |
| 2 | task per connection | 64 | 360.6k / 6,016 | 356.5k / 5,969 | −1.1% | 2 of 5 |
| 2 | task per connection | 256 | 365.6k / 6,005 | 369.9k / 5,955 | +1.2% | 5 of 5 |

r1 removes 46–56 instructions per request in every configuration, about 0.8%. Throughput is within noise except in two configurations where r1 was faster in all five pairs, by 1.8% and 1.2%; the other six are level or mixed. r2 was not run against HTTP.

## Decision

Reject both as marginal. r1's gain is repeatable in instructions but not consistently in throughput, and its lock skip adds a SeqCst handshake that every later change to `Notification` or `Direction::wait` must preserve. r2 is no simpler than the existing two updates and is not faster. After r1, the remaining locked-instruction gap to Tokio is expected to be mostly task machinery: running and waking an `async_task` task, and the global outstanding-runnable counter updated on every schedule and retirement.
