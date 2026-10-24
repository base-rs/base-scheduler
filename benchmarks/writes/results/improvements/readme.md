# Reducing ordered writer overhead

The ordinary task-work policy measured here was subsequently withdrawn after [fresh Tokio parity controls](../../../echo/parity.md) showed a substantial regression in concurrent borrowed-monitor traffic. Production restores cooperative delivery for unused thread-bound worker rings. The cancellation allocation reductions remain; the ordinary-delivery kernel-entry and throughput results below describe the archived experiment.

This follow-up keeps the writer's ownership, ordering, cancellation and submission acknowledgement contracts. It removes unnecessary cancellation setup and changes worker rings to ordinary kernel completion delivery.

## Changes

`Cancellation::is_never()` identifies signals which cannot be cancelled. Stalled io_uring writes with such signals now omit both the boxed cancellation future and the cancellation wake allocation. Actual cancellable writes retain their listener and reuse the wake allocation when `Arc::get_mut` establishes exclusive ownership. A delayed notification may retain an older wake object; that case allocates a new object rather than changing the older request ID. Unique request IDs also prevent already-queued old cancellation commands from affecting new writes. Explicit writer close and drop still cancel accepted output independently of its submission token.

Worker rings now retain the ordinary ring created during selector setup. Removed code includes optional cooperative-ring replacement, its failure fallback, the started flag and the taskrun-only flush branch. CQ overflow still forces kernel progress. Submission receipts still wait until the kernel consumes the first SQE, and a subsequent submission still completes its predecessor before accepting another buffer.

The flag distinction matters: `COOP_TASKRUN` suppresses forced userspace interruptions and permits completion work at ordinary kernel transitions. It does not restrict processing exclusively to `GETEVENTS`; `DEFER_TASKRUN` provides that stronger deferral. The former driver's `TASKRUN_FLAG` checks explicitly entered the kernel to publish pending completion work. Ordinary delivery allows that work to be processed during application computation or normal syscall return, which can eliminate later empty flush calls. This tradeoff follows the [official liburing setup documentation](https://github.com/axboe/liburing/blob/master/man/io_uring_setup.2); its performance depends on the application.

## Evidence

All measurements use Aiko, the shared performance lock, optimized builds, the frozen validating client and disjoint server/client CPU affinity. The initial ring experiment changes only its setup flags; the final implementation also removes the now-unnecessary setup machinery and includes cancellation allocation changes. Capacities and payload generation remain matched.

The cancellation-only comparison (`cancellation.json`) finds no reliable throughput change: writer medians are 1,880 responses/s before and 1,862 afterwards, with overlapping ranges of 1,808–2,208 and 1,861–2,184. The unchanged monitor control also varies. Retain this change for the known reduction in per-write allocation, rather than claiming a measured throughput gain from these twelve samples.

The isolated ordinary-taskwork confirmation (`taskwork-confirmation.json`) uses five repetitions of the single-worker, single-connection 256 KiB × 8 response with work=8 and a requested 64 KiB send buffer. The writer median improves from 1,878 to 2,043 responses/s, or 8.8%. The five repetition-matched comparisons are all positive, but their sizes vary substantially.

## Kernel entry classification

Perf tracing records each `io_uring_enter` argument tuple, distinguishing submission-only calls, calls waiting for completions and empty `GETEVENTS` calls. The constrained-buffer single-connection case has the following counts per completed response. Startup and the final measurement boundary contribute small residual counts. These instrumented runs diagnose mechanisms; their throughput is not a capacity estimate.

| Path | Submission only | Empty `GETEVENTS` | Completion waits | Total entries |
| --- | ---: | ---: | ---: | ---: |
| Original monitor | 0.00 | 0.00 | 13.07 | 13.07 |
| Original writer | 8.00 | 6.57 | 5.69 | 20.26 |
| Flag-only writer prototype | 8.00 | 0.00 | 2.00 | 10.00 |
| Final combined writer | 8.00 | 0.00 | 1.82 | 9.82 |
| Final monitor | 0.00 | 0.00 | 13.02 | 13.02 |

The final writer uses 51.5% fewer explicit ring entries than the original writer. Its submission acknowledgements remain visible as approximately eight submission-only entries. Empty completion flushes disappear, and fewer separate waits are needed. This is a reduction in explicit entries, not proof that total kernel work halves: ordinary delivery can interrupt application execution instead. The manifest retains exact argument histograms, response counts, script contents and hashes, and the paths, sizes and hashes of the large raw traces kept on Aiko. Empty `GETEVENTS` describes the observed arguments; attribution to the former taskrun-flush path follows the driver logic and this bounded workload's lack of CQ overflow.

## Final capacity comparison

The combined implementation is then compared against the original writer in sixty samples: three repetitions of each writer and monitor across five cases. The first four cases are randomized together in `comparison.json`; the default-buffer 64 KiB case is a subsequent randomized twelve-sample supplement in `supplement.json`. Each sample has one second of warmup and three measured seconds. Values below are median complete responses per second; `constrained` requests a 64 KiB send buffer, which Linux reports as 128 KiB.

| Workers / connections; response | Writer before | Writer after | Change | Monitor before | Monitor after |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 / 1; 64 B, work=0, default | 273,399 | 272,572 | −0.3% | 274,095 | 272,370 |
| 1 / 1; 64 KiB × 8, work=8, default | 8,761 | 8,578 | −2.1% | 8,753 | 8,742 |
| 1 / 1; 256 KiB × 8, work=0, constrained | 1,899 | 2,126 | +11.9% | 1,828 | 2,201 |
| 1 / 1; 256 KiB × 8, work=8, constrained | 2,139 | 1,996 | −6.7% | 1,821 | 2,114 |
| 4 / 32; 256 KiB × 8, work=8, constrained | 8,001 | 7,936 | −0.8% | 8,457 | 8,725 |

The extra-generation single-connection case is inconsistent across runs. A subsequent five-repetition writer-only confirmation (`combined-confirmation.json`) measures 1,983 before and 2,044 afterwards, or +3.1%, instead of the earlier −6.7%. Sample ranges overlap substantially: 1,879–2,295 before and 1,934–2,274 afterwards. This does not establish either a reliable gain or regression for that workload. It also demonstrates why the isolated +8.8% result alone cannot characterize the final implementation.

The backpressure case without added generation work has a clearer gain in this comparison: all three final writer samples exceed all three baseline samples. Small immediate writes and the many-connection writer case are within 1% of their baselines. The default-buffer generation case has a 2.1% lower median with overlapping sample ranges. Keep these modest losses visible; eliminating allocations and kernel entries does not guarantee that every application workload gets faster.

## Same-executable policy control

To separate policy from executable-layout differences, an experimental build selects normal or cooperative ring setup through a startup-only environment variable. Both modes use the same executable, retain the taskrun check and run the same writer code. No environment switch or public option is added to the production library. `policy-control.patch` records this temporary experiment; `policy-control.py` and `policy-control.json` retain its runner, startup environment, hashes, shuffled schedule and all eighteen samples. The kernel-mode selection happens during scheduler setup, before running connection tasks and outside the measured loop.

| Workers / connections; 256 KiB × 8, constrained | Cooperative median | Normal median | Normal change |
| --- | ---: | ---: | ---: |
| 1 / 1; work=0 | 1,905 | 1,981 | +3.99% |
| 1 / 1; work=8 | 2,101 | 1,977 | −5.89% |
| 4 / 32; work=8 | 8,057 | 8,053 | −0.05% |

For work=0, all three normal samples exceed all three cooperative samples. The four-worker result is effectively unchanged. Work=8 remains inconclusive: normal samples are 1,958, 2,282 and 1,977; cooperative samples are 2,226, 1,882 and 2,101. Their arithmetic means are nearly identical (2,072.2 versus 2,069.7, or +0.12%), despite the negative median comparison. Reporting both statistics makes that small sample's sensitivity visible; neither establishes a general generation-workload advantage. The host's recorded power policy remains unchanged.

Keep ordinary completion delivery as the simpler default for this prototype: it eliminates a demonstrated source of explicit kernel entries and has a repeatable benefit in the backpressure case without extra generation work. Keep direct sends and the cancellation allocation reduction. Do not claim a universal throughput improvement or recommend the writer over borrowed monitors for every workload. The generation-heavy results, the default-buffer loss and the many-connection echo loss remain part of the evidence; real protocol workloads should determine any later policy change.

## Ordinary owned echo

Twenty-four separate echo samples (`echo.json`) compare ordinary owned receives/sends, rather than the new writer. They use 64-byte messages, one worker, three repetitions and the frozen echo client.

| Connections | Native before | Native after | Raw Tokio before | Raw Tokio after |
| --- | ---: | ---: | ---: | ---: |
| 1 | 225,977 | 239,546 | 275,391 | 277,041 |
| 32 | 630,648 | 617,895 | 709,387 | 722,795 |

Native moves +6.0% for one connection and −2.0% for 32. All three final single-connection samples exceed all three baseline samples; the 32-connection ranges overlap. The raw Tokio control moves +0.6% and +1.9%. This short control does not establish a general echo throughput improvement, and the ordinary owned path still trails raw Tokio. It guards against treating a writer-only gain as proof about unrelated operations.

## Verification

The full macOS Bake gate passes at `4e3fcd9`: 23,922/23,922 measured regions, with three invariant regions excluded. The final full Linux workspace gate passes at `5990901` (root equivalent `72fdd77`): 34,532/34,532 measured regions, with twelve invariant regions excluded. The later root `519567e` removes only an unnecessary `mut` annotation in a test. The Linux-only ring cleanup does not change macOS behavior.

Independent review approved both production changes and confirmed preservation of queued and in-flight work, worker migration, retired workers, submission acknowledgement, cancellation and overflow handling. The delayed-notification regression test exercises both exclusive wake reuse and surviving older notifications.

Formatting, strict workspace Clippy across all targets and features on macOS and Linux, Rust documentation with warnings denied, macOS Rust 1.85.1 package checks and the Linux-target io_uring test check pass. The test-only mutable-binding removal and fixture alias address lint clarity and change no reachable behavior. No Windows execution results are claimed.

## Retained artifacts

- `manifest.json` contains all sixty final comparison samples and their two metadata groups, derived summaries, kernel-entry classification, and the scripts and remote locations for large raw traces.
- `comparison.json` and `supplement.json` retain the original capacity runner outputs separately. `combined-confirmation.json` retains the additional ten writer-only samples.
- `cancellation.json` isolates cancellation-allocation changes; `taskwork-confirmation.json` is the earlier ten-sample flag-only confirmation.
- `enter-classification.json` retains the initial diagnostic sample metadata; the parsed original and final counters are in `manifest.json`.
- `echo.json` and `echo.py` retain the ordinary owned-I/O regression control.
- `policy-control.json`, `policy-control.py` and `policy-control.patch` retain the final same-executable experiment. The temporary environment switch is not shipped.

All capacity comparisons use the frozen validating client and optimized binaries. Interpret the separate experiments individually instead of pooling repetitions from different source revisions or measurement windows into one favorable number.
