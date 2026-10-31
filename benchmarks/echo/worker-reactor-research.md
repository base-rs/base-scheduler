# Worker/reactor performance research

Baseline: main `5cc179985803e1da812353df73868db668234202`, including post-submission CQ publication. The [execution model](../../context/execution.md) maps costs and queueing stages to source; the [progress requirements](../../design/worker-progress.md) define preserved behavior. This ledger distinguishes observations from hypotheses.

## Starting evidence

Measurements use Hana, an Arch Linux 7.2 host with a four-core, eight-thread Intel Xeon E3-1240 v6.

The preceding raw hello-world experiment on Hana compared equivalent owned reads before/after post-submit CQ collection. At 240,000 offered requests/s, five paired rounds improved median actual-send RTT from 127.6 to 74.8 us; paired p99 changes were inconsistent. Separate unrestricted capacity at 64 connections increased from 250,626 to 257,767 requests/s. The same experiment's Tokio control had lower actual-send RTT; the remaining tail gap is unexplained. These are historical observations, not acceptance targets or new measurements. Detailed evidence is retained in the companion base-http repository under `benchmarks/hello/results/post-submit-20261028/`.

The raw server uses one `FuturesUnordered` supervisor for all connections. That limits what executor-task budgeting can explain. Instrumented prefixes showed payload deferral in about 1–2% of prepared payload batches, so its presence alone does not explain the full gap.

## Experiment 1: owned payload submission deferral

**Hypothesis:** excluding payload and cancellation SQEs from the nonblocking deferral rule removes stage-to-issue delay behind another application batch. The cost may be extra kernel entries, especially at low concurrency. There is no prediction that it will close the remaining p99 gap.

**Isolated variant:** retain bounded deferral for newly staged readiness-only batches (persistent monitors and transient `PollAdd` requests); issue newly staged payload/cancellation batches on that turn. Keep the worker policy, CQ publication, readiness semantics and payload ownership intact.

**Validation plan:** regression tests distinguish staging from issuance; retain readiness-only bounded deferral, submission-error retry and shutdown tests. Measure uninstrumented before/after owned raw servers and unchanged borrowed and Tokio controls on idle Hana. Use five interleaved randomized rounds at 240,000 offered requests/s with 64 connections, then a separate unpaced cohort at 1 and 64 connections. Retain source patches, binaries, checksums, environment, per-sample data and decisions. Use separate diagnostic builds for boundary frequency if needed; never include their timing in production comparisons.

**Decision: reject the prototype as the default.** At 64 connections, the paired throughput change is +0.54% with mixed signs. At one connection, throughput improves 1.75%, but process CPU/request rises 5.65% in the paired median and is worse in all five pairs (+5.07% to +10.83%). Fixed-rate latency does not establish a consistent improvement: paired RTT p99 changes -7.40% with a -22.15% to +54.98% range; scheduled-arrival p99 changes +11.52% with a very broad mixed range. No numerical threshold was invented to force this choice; these results do not justify more kernel entries and another hot-path classification rule.

Five interleaved randomized rounds at 240,000 offered requests/s validate all 48 million responses including warmup. All samples deliver within 0.043% of the target; every planned measurement request is retained, including late results. A separate five-round unpaced cohort measures capacity at 1 and 64 connections. The source, candidate patch and frozen binaries remain available for reproduction. The [measurement record](https://github.com/base-rs/base-scheduler/tree/b515ad0928bd8637c05a7abda046abbfdd1c6f5c/benchmarks/echo/results/worker-reactor) in #66 includes per-sample data, environment, diagnostic patches, checksums and verification; it is not retained on main. The retained implementation change corrects the misleading readiness-only comments and adds a bounded-owned-read deferral regression. It also removes batching-dependent timing assumptions from two lifecycle fixtures; it does not change the runtime submission predicate.

Medians of individual runs (RTT and scheduled latency are distinct):

| Metric | Baseline owned | Immediate payload prototype | Borrowed control | Tokio control |
| --- | ---: | ---: | ---: | ---: |
| 240k/s RTT p50, us | 58.271 | 58.367 | 33.855 | 29.967 |
| 240k/s RTT p99, us | 308.223 | 265.471 | 268.799 | 261.759 |
| 240k/s scheduled p99, ms | 2.230 | 2.916 | 2.664 | 3.019 |
| Unpaced 64-connection requests/s | 262,015 | 263,380 | 241,785 | 248,719 |
| Unpaced 64-connection CPU us/request | 2.690 | 2.663 | 2.935 | 2.834 |
| Unpaced 1-connection CPU us/request | 6.653 | 7.116 | 6.222 | 5.825 |

These results are a new cohort, not directly comparable latency samples from the preceding study. Its controls and baselines also vary. In particular, differences between medians are not the paired median changes, and saturated p99 values at different throughput do not establish equal-load tail behavior.

## Experiment 2: spacing readiness worker turns

Baseline: main `aaf337eab1d0fc99e6177a61649dfc94fe6b30f1`.

**Hypothesis:** a busy worker's zero-timeout turn between ready batches is a system call on epoll and kqueue. Spacing those turns removes that cost where consecutive small batches are common. A single-worker timer-and-yield loop takes 760 ns per iteration on Native readiness, against 359 ns on Tokio in the same run.

**Isolated variants:**

- turn after 61 task polls, as Tokio polls I/O after 61 ticks;
- the same on one FIFO per worker, with each batch counted before polling it instead of alternating two queues, with and without a 61-poll batch cap;
- skip zero-timeout turns after an empty poll, with backoff.

**Validation:** interleaved base-http hello cohorts at one and two workers, with supervisor and task-per-connection servers, measuring user instructions, cycles and throughput; plus the Native timer suite.

**Decision: reject all three.** The timed variants bring the yield loop to about 255 ns, but HTTP serving on Hana either does not change or regresses by 1–7%:

- At 64 connections, two workers with a task per connection gain about 150 instructions per request under every interval variant.
- The adaptive skip keeps instructions level but costs cycles and throughput.
- The one-FIFO counted batch shows no gain over the alternating queues.

The [measurement record](results/turn-interval/readme.md) retains the patches, per-sample data and method.

## Experiment 3: readiness publication atomics

Baseline: main `aaf337eab1d0fc99e6177a61649dfc94fe6b30f1`.

**Hypothesis:** Native readiness spends about 21 locked instructions per echo round trip, against Tokio's 13. Publishing a direction takes two or three atomic updates and then locks its notification mutex even when nobody waits. Fewer atomics should show in instructions and throughput.

**Isolated variants:**

- one compare-and-swap for the generation, readiness and sticky flags, plus a waiter count that lets publication skip the lock when nobody waits;
- the compare-and-swap alone.

**Validation:** interleaved echo runs on `native-poll` with one worker and 32 connections, counting locked loads; and interleaved base-http hello cohorts at one and two workers for the first variant.

**Decision: reject both as marginal.** The first variant removes about three locked instructions per echo round trip and 46–56 user instructions per HTTP request, but throughput is within noise in six of eight configurations, and its lock skip adds a SeqCst handshake that later changes must preserve. The compare-and-swap alone removes two locked instructions but adds about ten instructions, and its echo throughput was below baseline in every pair.

The [measurement record](results/readiness-publication/readme.md) retains the patches, per-sample data and method.

## Diagnostic boundaries

Separate instrumented executables count prepared/deferred payload requests and measure staging-end to the next kernel-entry attempt. That interval excludes the call itself and is not a kernel consumption timestamp. One in 128 owned reads separately samples queue publication to entry preparation, and result publication to consumption under the existing completion-state lock. Histogram limits are 1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024 and 2048 us, then overflow. Reports are cumulative prefixes every 4096 turns, not whole-run totals.

The first retained baseline prefix deferred 605 of 118,176 prepared payload batches (0.51%), containing 38,713 of 1,169,796 payload requests (3.31%). Its staging-to-next-entry-attempt interval averaged 0.540 us, with maximum 5.572 us. The direct delays sampled before preparation and after publication were much larger: mean 19.87 us over 9,139 queue samples and 22.44 us over 9,138 consumption samples. The prototype eliminates deferrals, but its corresponding diagnostic means are 25.28 and 26.30 us; those instrumented runs do not demonstrate a reduction in overall queueing delay.

The hypothesis that this deferral was routinely hiding substantial application work before issuance is not supported by this diagnostic. The larger observed boundaries guide the next investigation, but their clocks and counters alter batching. Do not sum them to explain the uninstrumented RTT or treat these means as production latency guarantees. Repeated diagnostic evidence and exact artifacts are recorded with the companion study. A pseudorandom-sampling control avoids repeatedly selecting the same positions in a periodic connection order: its three baseline prefixes average 0.630–0.708 us for deferral, 28.59–34.91 us queue-to-preparation and 30.40–35.92 us publication-to-consumption. This retains the difference in scale, while the changed means underline the sensitivity of instrumented batching. It does not identify a specific executor fix or explain production RTT.

A queued receive may be prepared before the next request arrives, so its queue interval can overlap client scheduling time. Establish input availability and response progress before treating that interval as response-critical delay. Result publication-to-consumption is more directly on the observed input's response path, but still includes CQ-batch and supervisor/executor effects which these probes do not separate.

## Research queue

The next experiment is selected from measured stage delays, rather than from Tokio's implementation choices alone:

| Boundary | Probe or workload | What would justify a change |
| --- | --- | --- |
| Staged payload to kernel issuance | Deferred batches and requests, elapsed time until the next kernel entry | A recurring delay with repeatable end-to-end benefit when removed |
| Published result to application consumption | Sample timestamps under the existing completion-state lock, collected on consumption | Material delay attributable to executor or supervisor polling, confirmed without probes |
| Ready batch to next driver/timer visit | Mixed self-waking and socket/timer workloads; distinguish independently spawned tasks from inner futures. Turn spacing by poll count, a one-FIFO counted batch and empty-poll backoff were rejected (Experiment 2) | Better cost/delay tradeoff while preserving external work, I/O and timer progress, with no HTTP regression at one and two workers |
| Demand-driven read/write to overlapping progress | Streaming full-duplex traffic and slow-peer backpressure with bounded owned storage | Useful overlap beyond the one-read/one-write control, without unbounded buffering |

Instrumentation is compiled into separate executables. It can change batching and CPU usage; its distributions locate possible delay and do not replace the uninstrumented latency/capacity cohorts. In particular, fewer ring entries is not by itself a success criterion: deferred task-work batching already showed that kernel work per entry also matters.
