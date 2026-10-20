# Native worker hot-path research

Checking the two admission queues before attempting a batch steal removes unnecessary synchronization from empty-queue searches. The first Aiko screen improves Native's single-worker, single-connection throughput by 2.8% for owned reads and 2.4% for borrowed reads; a separate five-repetition confirmation gives 3.1% and 2.3%. Concurrent workloads remain within the screen's noise. The change retains the existing task ownership and parking handshake.

The [validated tables](results/worker-hotpath/tables.md), [raw evidence and verification notes](results/worker-hotpath/readme.md), and [summary generator](results/worker-hotpath/summarize.py) retain both isolated experiments.

## Empty admission queues

The worker admits external work from the global injector and its own incoming injector at every batch boundary. Crossbeam's `Injector::steal_batch` enters a heavier path, including a sequentially consistent fence, before discovering an empty queue. `Injector::is_empty` uses two sequentially consistent loads. Checking it first avoids that heavier path when these queues are empty. Actual deque stealers remain unchanged; their own emptiness check already includes a fence.

The worker still publishes its sleeping state and performs the existing fence and fresh work search before parking. A racing external enqueue is either observed by that search or retains an unpark and driver notification. Active workers continue checking external work at every batch boundary. No task ownership, cancellation, shutdown accounting, or I/O behavior changes.

Three randomized repetitions per cell give these median completed echo exchanges per second:

| Workers/connections | Owned before → candidate | Borrowed before → candidate | Unchanged Tokio before → candidate control |
| --- | ---: | ---: | ---: |
| 1/1 | 239,197 → 245,841 (+2.78%) | 238,737 → 244,481 (+2.41%) | 274,191 → 271,897 (−0.84%) |
| 1/32 | 635,654 → 637,337 (+0.26%) | 572,076 → 572,864 (+0.14%) | 702,600 → 698,707 (−0.55%) |
| 4/128 | 2,088,534 → 2,100,424 (+0.57%) | 2,118,763 → 2,123,483 (+0.22%) | 2,137,806 → 2,135,390 (−0.11%) |

Single-connection owned ranges do not overlap in this screen: 235,136–239,848 before and 243,157–250,421 after. Borrowed ranges overlap widely. The concurrent differences are too small to claim a reliable improvement. Tokio is the original ordinary-read control; these rows do not equalize its receive policy with Native or isolate interface dispatch.

A separate randomized 25-sample confirmation uses five repetitions at one worker/connection with the same frozen optimized binaries. Owned Native rises from 238,139 to 245,560 requests/s (+3.12%); borrowed rises from 231,411 to 236,820 (+2.34%). The unchanged Tokio control reaches 271,754 requests/s. The direction repeats, although the confirmation ranges overlap and neither screen establishes a precise universal percentage. [Confirmation table](results/worker-hotpath/tables.md#five-repetition-single-connection-confirmation).

## Rejected single-worker notification shortcut

A separate variant skips idle-worker notification after a successful same-worker enqueue when there is exactly one worker. That worker is already executing and checks its own queue before parking; remote and multiworker notifications remain intact. The screen establishes no useful broad improvement: owned throughput changes by −0.45%, −0.22%, and −0.35%, while borrowed changes by +2.31%, +0.18%, and −0.13%. The single-connection ranges overlap widely. This special case is archived and is not applied to production.

## Verification and limits

The reviewed admission-queue change passes Native's macOS and Linux Bake coverage gates: 1,337/1,337 measured source regions, including the full Native test suite and doctests. Formatting and all-target Native Clippy with warnings denied pass. The reviewed patch only strengthens the idle-driver test fixture; the measured production change is identical. Linux results are recorded with the [follow-up verification](results/worker-loop-followup/linux-verification.json).

All builds and measurements acquire the exclusive Aiko performance lock. Builds use locked dependencies and Cargo's default optimized release profile without LTO or target-CPU overrides. Each sample uses one second of warmup and three seconds of measurement, persistent validated IPv4 loopback TCP connections, `TCP_NODELAY`, a reusable server buffer, one outstanding 64-byte exchange per connection, server CPUs 0 through workers−1, and client CPUs 4–7. CPU values in the raw evidence cover process lifetimes. Short loopback screens do not establish sub-percent equivalence or predict general application and physical-network performance.
