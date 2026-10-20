# Readiness submission research

This study tests reducing io_uring submission overhead without moving buffer preparation or changing buffer storage. The [raw results and validation script](results/readiness-submission/readme.md) retain all variants, including rejected implementations. The baseline is merged PR #18, revision `8400db7`.

## Distinguish readiness from buffer callbacks

Preparing an owned request may call user-provided buffer accessors. Those accessors can wake tasks, so the driver must return to the executor before waiting. Native deliberately avoids interrupting its own executing worker: a buffer callback cannot rely on a self-interrupt to rescue an incorrect wait.

A readiness request only accesses fixed native socket metadata. It can safely reach submission and waiting in the same turn. The experiment adds an internal `entry_may_wake` method, conservatively true by default and false only for readiness requests. Accepted submissions alone no longer mark the driver productive; actual callback preparation, cancellation, rejection, and completions still do. A full command batch always returns because unprocessed local commands have no eventfd interrupt.

This avoids the earlier combined-wait prototype's cached scalar pointers and early message preparation. Request ownership, original-completion retirement, buffer representation, allocation policy, and the public capability interface are unchanged.

## Moving the worker turn penalizes owned I/O

The first screen compares the baseline, moving the nonblocking driver turn before the next ready batch, and that worker change combined with readiness-only submission/waiting. Three randomized repetitions give:

| Workers/connections, 64 bytes | Owned with worker change only | Owned with readiness optimization | Borrowed with readiness optimization |
| --- | ---: | ---: | ---: |
| 1/1 | −6.99% | −6.33% | +7.10% |
| 1/32 | −1.42% | −1.22% | −0.17% |
| 4/128 | +0.21% | −0.22% | −1.28% |

The worker-only control exposes the problem: owned requests still require a submit-only turn after preparing buffers. Moving that turn into the idle path adds a work search and sleeping-state handshake. Readiness requests can bypass that extra turn, but owned requests cannot. This variant remains archived.

## Stage readiness without moving the worker loop

The second variant retains the original worker loop. A nonblocking turn can stage one readiness-only batch when its SQ initially has no queued entries. The next idle wait submits that batch and waits together. If tasks remain busy, the next nonblocking turn sees existing SQ entries and submits them; readiness cannot be deferred indefinitely. CQ overflow and deferred kernel task work always force kernel progress.

Three randomized repetitions across the full matrix give:

| Workers/connections/bytes | Owned change | Borrowed change |
| --- | ---: | ---: |
| 1/1/64 | +2.70% | +1.46% |
| 1/32/64 | −0.04% | −1.17% |
| 4/128/64 | −0.11% | −0.51% |
| 1/1/4096 | −0.50% | −2.40% |
| 4/128/4096 | −1.34% | +0.04% |

Single-connection ranges overlap, and the small owned improvement is not an established benefit. Borrowed throughput at 1/32 falls from 571,804/s to 565,138/s, with non-overlapping ranges in this screen. This is not a broad performance win.

Separate diagnostic counter runs establish that staging cuts borrowed-path `io_uring_enter` calls from 2.000 to 1.000 per completed exchange. Instructions fall from 27,825 to 27,035, but cycles are essentially unchanged at 15,534 versus 15,508. Owned I/O retains 2.000 enter calls and changes from 26,539 to 26,624 instructions. These are instrumented diagnostics, not uninstrumented capacity measurements. Fewer enter calls do not by themselves establish better application throughput.

## Publish completions before returning from a wait

The final variant combines staging with collecting the available CQ snapshot immediately after a blocking wait. It keeps the original worker loop and request representation. This avoids another empty worker search and idle-driver turn before publishing completed tasks. Completion processing still publishes the whole CQ snapshot through registered wakers; application futures run only from the ready queues.

Three randomized repetitions give the following medians, in completed exchanges per second:

| Workers/connections/bytes | Owned before → after | Change | Borrowed before → after | Change |
| --- | ---: | ---: | ---: | ---: |
| 1/1/64 | 240,014 → 251,548 | +4.81% | 239,848 → 240,554 | +0.29% |
| 1/32/64 | 635,522 → 635,366 | −0.02% | 572,266 → 571,821 | −0.08% |
| 4/128/64 | 2,082,127 → 2,091,067 | +0.43% | 2,127,888 → 2,132,282 | +0.21% |
| 1/1/4096 | 232,650 → 231,536 | −0.48% | 234,437 → 233,682 | −0.32% |
| 4/128/4096 | 1,856,997 → 1,859,049 | +0.11% | 1,870,549 → 1,883,197 | +0.68% |

Most changes are small relative to the repetition ranges. The 1/1/64 owned ranges overlap at 241,207–242,303/s, so its 4.81% median gain needs longer confirmation. The 4/128/4096 borrowed ranges do not overlap in this screen: 1,861,771–1,876,994/s before and 1,880,402–1,883,776/s after. Every negative median change has overlapping ranges. This screen finds no material throughput regression, but cannot exclude smaller changes.

Separate counter runs support the reduction in driver work:

| Backend | `io_uring_enter` calls/exchange before → after | Instructions/exchange before → after | Cycles/exchange before → after |
| --- | ---: | ---: | ---: |
| Owned | 2.000 → 2.000 | 26,534 → 26,181 | 15,271 → 15,001 |
| Borrowed | 2.000 → 1.000 | 27,806 → 26,581 | 15,950 → 14,885 |
| Raw Tokio control | 0.000 | 23,240 | 13,609 |

These instrumented runs use two repetitions of the 1/1/64 case, independently of the capacity matrix. Borrowed instructions fall about 4.4% and cycles about 6.7%; owned instructions fall about 1.3% and cycles about 1.8%. Counter instrumentation changes scheduling and throughput substantially, so these percentages describe the diagnostic runs rather than a promised capacity gain.

## Decision and remaining work

Retain the final combined variant. It eliminates an `io_uring_enter` call on the measured borrowed path and reduces measured driver work without the worker-order prototype's regression. The change adds no persistent state, allocations, unsafe storage, or public API. One bounded staging rule and an extracted CQ collector replace the extra submit/wait separation and completion-delivery turn. Retain the original worker loop; archive the worker-order and staging-only variants for reproducibility.

This does not close the single-worker Tokio gap. In the final capacity matrix, owned and borrowed 1/1/64 are respectively 6.8% and 10.9% below the ordinary-read Tokio control; 1/32/64 remains 9.2% and 18.3% below. At 4/128/64, owned is 1.7% below that control and borrowed is 0.3% above. The receive policies differ, so these are workload-level comparisons, not interface-dispatch costs.

The next research target is the lifecycle of a readiness request: the boxed request, completion channel, capacity admission, identifier assignment, and in-flight map. The current implementation creates and retires that state for each pending readiness wait, while Tokio can reuse a socket registration. This study does not isolate or quantify each component's cost. A follow-up should profile those steps and compare an isolated reusable-registration prototype, preserving cancellation, migration, shutdown, and original-completion ownership.

## Verification and measurement scope

The staging-only variant passes Linux io_uring coverage at 2,141/2,141 measured regions; the combined variant passes at 2,152/2,152. One existing invariant-only region is excluded by the shared gate. Both pass all-feature/all-target Clippy with warnings denied and formatting. The combined gate runs 84 unit tests, 180 conformance tests, and one documentation example. The [verification record](results/readiness-submission/verification.json) preserves commands, results, and source/log identities. Regressions verify same-turn readiness submission/waiting, bounded nonblocking deferral, remaining local commands after a full batch, callback-induced wakes, same-turn completion publication, preserved explicit wake permits, and post-wait notification errors. The final variants only change Linux io_uring code.

Every Aiko build, test, benchmark, and profile acquires the exclusive performance lock. All variants use locked dependencies, Rust 1.99.0 / LLVM 23.1.1, and Cargo's default optimized release profile without LTO or target-CPU overrides. Timer slack is inherited consistently within this study. Servers use CPUs 0 through workers−1; the unchanged frozen client uses CPUs 4–7. Each capacity sample has one second of warmup and three measured seconds, persistent validated IPv4 TCP messages, and `TCP_NODELAY`.

Matrices contain three randomized repetitions per case. Short loopback studies cannot establish universal sub-percent changes or predict physical-network applications. Ordinary-read Tokio is a capacity/noise control, rather than an equalized receive-policy comparison or a measurement of interface dispatch overhead.
