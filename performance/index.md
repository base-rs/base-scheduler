# Performance Investigations

Measured and rejected optimization attempts on base-scheduler, kept so that they are not retried without new evidence. Each record follows the [base-benchmark investigation convention](https://github.com/base-rs/base-benchmark/blob/main/context/investigations.md) and links its raw results under `benchmarks/*/results/`. Records written before 8 November 2026 were reconstructed from a working log and the pull requests that adopted the changes; where a measurement was not retained, the record says so.

Unless stated, measurements are screens on hana (Intel Xeon E3-1240 v6, 4 cores, 8 threads). Screens are triage, not acceptance evidence.

## Investigations

| Investigation | Decision | Summary |
| --- | --- | --- |
| [Runnable counter](investigation-2026-11-04-runnable-counter.md) | adopted, rejected | Padding `Shared::outstanding` (#94) made contended yields 7% faster. Per-thread counters cost more or changed nothing. |
| [Timer registrations borrow the scheduler](investigation-2026-11-05-timer-registration-borrow.md) | adopted | `sleep yield` 64/4 went from 3% slower than `main` to 8–10% faster. |
| [Stall rescue monitor costs](investigation-2026-11-05-stall-rescue-monitor-costs.md) | adopted, rejected | The monitor costs a few percent of CPU on io_uring and no lateness; it did not cause the p50 rise. |
| [Owned stream readiness](investigation-2026-11-06-owned-stream-readiness.md) | adopted | Owned echo at 1 worker +19–23% (#104). |
| [Driver wakes](investigation-2026-11-06-driver-wakes.md) | adopted, rejected | Driver-local wakes cut 4-worker CPU/request 1.6–3.3% (#105); four other wake and steal policies did not help. |
| [Tokio adapter reads](investigation-2026-11-06-tokio-adapter-reads.md) | adopted | Clearing after short reads and a compact path took the adapter from 18% to 4–7% behind raw Tokio (#107). |
| [Stream transfers and one receive rule](investigation-2026-11-06-stream-transfer-registration.md) | adopted | Cached registrations for pooled and vectored transfers (#108), then one receive rule for every consuming receive (#110). |
| [An apparent io_uring HTTP regression](investigation-2026-11-06-iouring-http-regression.md) | rejected | Not established: `main` matched `v0.4.0` while a copy of `v0.4.0` moved 2–4%. |
| [Timer batching](investigation-2026-11-06-timer-batching.md) | rejected | The periodic diagnostic's p50 rise is mostly its own feedback; deferring idle wakes was a trade-off. |
| [Multi-worker HTTP](investigation-2026-11-07-multi-worker-http.md) | adopted, open | Keeping io_uring polls on their ring made it 14–26% faster at 2–4 workers (#109); Native's p99 at 4 workers is open. |
| [Native CPU variation](investigation-2026-11-08-native-cpu-variation.md) | open | CPU per expiry varies between processes of one executable: Native's more than threefold; io_uring's and the Tokio adapter's 20–35% in `expire-contended`. |

## Open ideas

Untested; each would need its own investigation.

- **Steal single queued tasks only from stalled workers.** The driver-wakes screen suggests about 1%.
- **Pad `Shared::idle_workers`,** which every park and unpark writes, onto its own cache line. Expected under 1%.
- **Packet and pipe transfers on the readiness selector** still use the generic path, and the packet reader's pooled `read` still makes the extra `poll(2)` calls.
- **A pooled `read` that would block registers two readiness listeners:** `Receive` clears readiness and calls `poll_wait`, the future is dropped, then `wait_readable` registers another. A receive could return `Pending` straight after the `WouldBlock` clear when the caller waits separately.
- **Owned vectored transfers whose buffers lack prepared I/O vectors,** such as plain arrays, allocate scratch per call. This may explain why owned vectored costs about 0.5 µs more per request than borrowed vectored.
- **The Tokio adapter's remaining 5–9% CPU gap to raw Tokio at 1 worker:**
  - cache the runtime `Id` beside `closed`, so that `check_socket` stops dereferencing the handle;
  - store the user future inline in `TaskFuture` instead of boxing it;
  - drop the separate `Arc<socket2::Socket>` behind every syscall;
  - check whether the waiter reservation's compare-and-swap per read matters.

  Per-operation `socket_result` handling and cancellation polling there are real but second-order; #107 bypassed them for `Cancellation::never()` rather than tuning them.

## Measurement notes

- **Build layout.** Identical source built as a different binary moved HTTP hello by 2–4% in one run and 0.5–1.1% in another ([io_uring regression](investigation-2026-11-06-iouring-http-regression.md)) and the timer diagnostic's p50 by 12.6% at 4 workers ([timer-batching](../benchmarks/timers/results/timer-batching/readme.md)), often with 0 of 5 rounds better. Before 8 November 2026, two kinds of control were labelled "layout" but were the same executable: `quick.py`'s echo control, and `base-benchmark` 0.1's replicas. Those measured run-to-run variation only; the [quick-screens readme](../benchmarks/echo/results/quick-screens/readme.md#controls) says which controls were genuine. `base-benchmark` 0.2 salts replicas so that they differ, and warns when one does not.
- **The periodic timer diagnostic** is bimodal and driven by its own feedback. Use 9 or more rounds, and prefer the spread mode for per-expiry costs.
- **wrk timeouts.** The Hyper control times out at 256 connections on one pinned CPU. Record socket errors per sample rather than aborting.
- **wrk limits multi-worker HTTP.** At 4 workers on hana it shares the server's SMT siblings, so multi-worker comparisons are at about the same load, not capacity.
- **Agents** must run benchmarks in the foreground: background notifications do not reach a subagent after its turn ends.

## Test reliability notes

Not performance results, but found while measuring:

- **Windows echo smoke timeouts.** `benchmarks/echo` smoke tests on Windows CI time out intermittently ("benchmark process timed out") on various readiness backends; rerunning passes. Not investigated.
- **`established_stream_reads_only_retry_stale_readiness_once`** in the Tokio adapter failed once in a full local `cargo test --workspace` on 7 November 2026 (Linux), and passed about 600 runs since, including 8 in parallel and under CPU load. The likely race: the test expects exactly one syscall from the next read, but any event Tokio dispatches for the socket between the `WouldBlock` and the clear, including writable or priority events since #107 added priority interest, makes the clear stale and adds a second attempt. Tracked in #115.
- **macOS loopback.** Delivery can lag a peer's write, so a test must wait for the bytes, for example with `peek` on the raw socket, before asserting that a syscall-first receive returns them. After a peer reset, macOS can let a waiting write complete before the error surfaces on a later write. Both made `main`'s macOS job flaky until #111.
