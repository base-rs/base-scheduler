# Readiness footprint

With one worker and 256 connections, Native echo ran about 4–5% below Tokio. The kernel cost per round trip was the same for both, about 15.4k cycles on the server CPU, so the gap was in user space, which accounted for 1,430 cycles against Tokio's 1,000. Native retired about the same number of instructions as Tokio but had twice the L1 data misses, and nine more locked instructions per round trip. Kernel work between `epoll_wait`, `recv` and `send` evicts most user lines, so every distinct line a round trip touches tends to miss. This change removes work and lines from the established read path, and makes reactor registration lazy for the fallback path.

| Change | Effect |
| --- | --- |
| Notify on transition | A wait registers only after observing its direction's ready bit clear, so the publication which sets that bit wakes every pending wait. Edge-triggered epoll reports `EPOLLOUT` with every `EPOLLIN`, so most publications find the bit already set and now skip the notification lock |
| Never-cancelled operations | Stream reads and writes with `Cancellation::never()` on a readiness worker skip the `cancellable` wrapper. On this path it already returned the operation after checking closure, but its future reserved space for two event-listener futures used by cancellable operations. Driver closure is still observed by the wait itself |
| Compact receive future | Established reads poll a hand-written `Receive` future, whose lazily created listener lives in its own state, instead of nesting async blocks that copy their arguments |
| Lazy fallback registration | `NativeSocket` wrapped `async_io::Async`, so every socket was also registered with async-io's epoll, adding a kernel wake callback per packet and a pointer chase to the descriptor. `RegisteredSocket` now owns the socket directly and registers a borrowed descriptor only when a fallback wait first needs it |

The scripts ran from `/tmp/claude-scripts` on Hana, with built binaries in its `bin` directory and the synchronized tree in `~/Developer/claude-bench/sched-gap`. The baseline `g0` is main `9621dfa` with the turn-spacing record's [echo backend patch](../turn-interval/echo-native-backend.patch); every echo variant carries that patch. `p1` adds notify-on-transition and the cancellation bypass, `p3` adds the compact receive future, and `p4` adds lazy fallback registration, which completes this change. `p2` is the rejected shared readiness word, `p5` the rejected inline first waiter and `p6` the rejected listener resume, described below. Echo binaries were built by [gbuild.sh](gbuild.sh) from a synchronized working tree, so intermediate variants have no commit of their own. The method and environment match the [readiness publication record](../readiness-publication/readme.md): Hana, server pinned to CPU 0, client on CPUs 1–3, `native-poll` with one worker and 64-byte messages, user-space `perf stat` counters per round trip.

## Echo

Medians of four interleaved repeats in [g-cohort10.txt](g-cohort10.txt), from [gcohort.sh](gcohort.sh), [echo-events.sh](echo-events.sh) and [gsum.py](gsum.py). Requests per second / user cycles / locked loads / L1 misses per round trip:

| Connections | g0 | p4 | Tokio |
| ---: | ---: | ---: | ---: |
| 1 | 64.5k / 2,283 / 25.5 / 68.6 | 66.0k / 2,001 / 23.5 / 55.0 | 65.3k / 2,773 / 28.7 / 80.1 |
| 32 | 216.3k / 1,266 / 21.3 / 44.8 | 223.2k / 1,094 / 19.3 / 37.7 | 223.4k / 1,070 / 13.1 / 23.3 |
| 256 | 205.8k / 1,446 / 21.2 / 47.7 | 212.0k / 1,203 / 19.2 / 39.2 | 215.2k / 1,008 / 12.1 / 21.8 |

p4 retires 1,217–1,222 user instructions per round trip at 32 and 256 connections, against g0's 1,423–1,427 and Tokio's 1,393–1,457. It matches or exceeds Tokio at 1 and 32 connections. At 256 connections the gap falls from 4.4% to 1.5%.

System-wide counts on CPU 0 at 256 connections, in [cpu0.txt](cpu0.txt) from [echo-cpu0.sh](echo-cpu0.sh), put p4 at 16.54k cycles per round trip against Tokio's 16.46–16.57k. Without the async-io registration, p4 spends about 170 fewer kernel cycles than Tokio, which offsets most of the remaining user-space difference. Precise L1-miss samples place most of that difference in the echo task's own future state, which spans about six cache lines because the layered `read_into` and `write` futures are laid out around their slow paths.

## HTTP

Five interleaved repeats of main (A) and p4 (P) with the base-http hello server, in [http-cohort-P.jsonl](http-cohort-P.jsonl) from [pcohort.sh](pcohort.sh), using the readiness publication record's `hbench.sh` and the turn-spacing record's `hbuild.sh`. Requests per second / user instructions / user cycles / process CPU µs per request:

| Workers | Server | Connections | A | P | Throughput change | P faster in |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | supervisor | 1 | 52.8k / 8,480 / 9,518 / 12.27 | 53.2k / 8,284 / 9,392 / 12.13 | +0.6% | 4 of 5 |
| 1 | supervisor | 64 | 182.9k / 5,885 / 4,191 / 5.42 | 184.2k / 5,690 / 3,912 / 5.37 | +0.7% | 4 of 5 |
| 1 | supervisor | 256 | 177.0k / 5,867 / 4,513 / 5.63 | 177.8k / 5,671 / 4,227 / 5.55 | +0.4% | 3 of 5 |
| 1 | task per connection | 1 | 52.3k / 6,701 / 8,085 / 11.32 | 52.9k / 6,480 / 7,309 / 11.07 | +1.1% | 4 of 5 |
| 1 | task per connection | 64 | 179.8k / 5,960 / 4,269 / 5.50 | 181.3k / 5,746 / 4,094 / 5.45 | +0.8% | 2 of 5 |
| 1 | task per connection | 256 | 170.5k / 5,996 / 4,528 / 5.84 | 175.0k / 5,777 / 4,192 / 5.62 | +2.6% | 4 of 5 |
| 2 | task per connection | 64 | 366.6k / 6,016 / 4,489 / 5.38 | 370.4k / 5,801 / 4,249 / 5.33 | +1.0% | 4 of 5 |
| 2 | task per connection | 256 | 379.9k / 6,008 / 4,502 / 5.24 | 378.7k / 5,790 / 4,387 / 5.24 | −0.3% | 2 of 5 |

P removes 195–221 instructions per request in every configuration, 2.3–3.7%, 1–10% of user cycles and up to 4% of process CPU. Throughput improves in seven of eight configurations; P was faster in 27 of 40 pairs. Median p99 latency is mixed, from 21% lower to 17% higher than A. These are closed-loop loopback measurements.

## Fixed rate

Five interleaved repeats of A and P with one worker at the same offered rate, in [fixed-rate-P.jsonl](fixed-rate-P.jsonl) from [fcohort.sh](fcohort.sh) and [fsum.py](fsum.py). `h2load --rps` paces each connection independently, so requests arrive in bursts and the 64 and 256 connection rates include queueing on the server CPU. h2load reports request time as mean, standard deviation and maximum, not percentiles. Medians, in µs:

| Server | Connections | Offered | A mean / sd / max | P mean / sd / max |
| --- | ---: | ---: | ---: | ---: |
| supervisor | 1 | 30k | 19 / 9 / 913 | 18 / 7 / 957 |
| supervisor | 64 | 128k | 273 / 135 / 3,490 | 281 / 128 / 2,330 |
| supervisor | 256 | 128k | 1,180 / 765 / 7,910 | 952 / 544 / 7,720 |
| task per connection | 1 | 30k | 18 / 11 / 3,000 | 19 / 11 / 1,260 |
| task per connection | 64 | 128k | 291 / 86 / 1,710 | 283 / 158 / 2,580 |
| task per connection | 256 | 128k | 1,030 / 645 / 7,890 | 842 / 511 / 7,500 |

Both sustained every offered rate. Latency is level at 1 and 64 connections. At 256 connections P's median mean is 18–19% lower, though the per-repeat means overlap, so this shows no latency regression rather than a firm improvement.

## Rejected

| Variant | Patch | Result |
| --- | --- | --- |
| Shared readiness word | [shared-readiness-word.patch](shared-readiness-word.patch), the driver change, which applies to main; p2 is p1 with this driver in place of notify-on-transition | Both directions in one atomic word, published with one update per event and notifying on transition. In [g-cohort6.txt](g-cohort6.txt) it saved 3 locked instructions per round trip over p1, but added 46–48 instructions and 5–6 L1 misses; throughput was mixed against main, −0.6% at 32 connections and +1.7% at 256, and every clear becomes a compare-and-swap loop |
| Inline first waiter | [inline-first-waiter.patch](inline-first-waiter.patch), against this change | An inline first slot in `Notification` avoids the waiter vector's heap line. In [g-cohort10.txt](g-cohort10.txt) it removed 2.7–3.9 L1 misses per round trip at 32 and 256 connections but added 1.4 at one connection, and throughput was level with p4: 212.1k against 212.0k at 256 connections, 221.2k against 223.2k at 32 |
| Resume through the listener | none | Completing a woken receive through `poll_wait`, rather than re-reading readiness at the top of its loop, made the post-wait path reachable without a race. In [g-cohort-p6.txt](g-cohort-p6.txt) it added 54 instructions and 3 L1 misses per round trip at 32 and 256 connections. The race is instead covered by a test hook that publishes before the wait |
| Skip writable publication | none | Skipping the writable direction's generation increment while it is already ready saved two locked instructions per event, but a racing `WouldBlock` clear could then erase a real edge and leave a writer waiting. Unsound |

Only Linux epoll was measured. The macOS kqueue driver and the io_uring selector, whose sockets also no longer register with async-io, are covered by tests but not by measurement.

Hand-written `read_into` and `write` futures, which would make the echo task's hot state contiguous, are the remaining footprint lever; their estimated gain of at most 0.5% did not yet justify the added code. The rest of the locked-instruction gap is task machinery: the global outstanding-runnable counter, `async_task` state transitions and the crossbeam deque.
