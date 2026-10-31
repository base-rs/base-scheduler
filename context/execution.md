# Worker and reactor execution model

This guide describes the Native worker and its worker-driven readiness and io_uring selectors. It explains where work happens and where progress can wait. It is an implementation model, not a latency guarantee or a prescription for other executors. See [the progress requirements](../design/worker-progress.md), [ownership contracts](design.md), and [I/O usage](io.md).

## Two performance questions

The cost of progress is the work needed to move an operation forward: native calls, allocation, reference counting, synchronization, routing, queue transfers, and application polling. Delay is the time an operation waits between those steps. Removing a syscall can reduce cost while delaying submission; publishing an already available completion promptly can reduce delay without another submission. Neither throughput nor a single latency percentile explains both.

Our working principle is to keep ownership local, amortize necessary kernel work, and avoid postponing useful progress across an unnecessary boundary. That principle motivates experiments; it does not decide every batching policy.

## The worker loop

The source of truth is `crates/native/src/worker.rs`:

1. `run_loop` services the worker's timer queue and checks shutdown.
2. `search` admits remote runnable work, tries the local ready queue, and then attempts stealing. A contention retry yields rather than parking.
3. With work available after an earlier task batch, the worker calls its driver with a zero timeout before running the next batch.
4. `run_ready_batch` freezes the existing ready queue. It polls its first task and the remaining frozen tasks; new wakes enter the next queue. Other workers can steal from both generations. This bounds a batch by membership, not time.
5. With no work, the worker advertises sleeping, fences, and searches again. Only then does it call the driver with the next timer timeout, or park if no driver remains. Publication and remote wakeup cover work arriving around that check.

These are per-worker boundaries, not global rounds. A completion collected on one worker may wake a task which another worker immediately steals. A local wake does not need an eventfd write to interrupt its own executing worker. An external wake of a sleeping worker uses the interrupt path.

The root of `block_on` is polled on the calling thread. A supervisor containing many connection futures is still one executor task: its inner polls are not individual ready-queue entries. No worker policy can preempt a single long poll. Consequently, a task-count budget alone would not bound the raw benchmark's `FuturesUnordered` supervisor. Timers and driver turns can both wait behind a long batch or a long individual poll.

## Operation stages and ownership

These are observations at different boundaries, not interchangeable meanings of “submitted.” Not every I/O path visits every stage.

| Stage | Meaning | Owner or progress mechanism |
| --- | --- | --- |
| Constructed | Lazy future exists; no operation has been admitted | Caller |
| Admitted | Backend accepted bounded work | Bound capability and selected backend |
| Queued | Local command queue or remote inbox holds accepted work | Selected worker/ring |
| Staged | An SQE has been prepared and published to the submission queue | Ring retains request storage before publication |
| Issued | Kernel entry has consumed the SQE | Original ring retains kernel-accessible owners |
| Completed | Kernel produced a result; it may still await collection | CQ / original ring |
| Collected and published | Driver handled the result and notified its waiter | Completion state and executor ready queue |
| Runnable | Executor accepted a wake; application may not yet have run | Local queue, remote inbox, or stealing worker |
| Consumed | Application poll observed the result | Caller |

Retirement is a separate lifetime boundary. A final original completion ends that submission's kernel access; a multishot intermediate CQE does not. Persistent monitor records also wait for any issued cancellation CQE before reuse. A cancellation acknowledgment alone cannot return a payload owner. Abandoning the future does not retire the accepted operation. See the detailed [ring ownership contract](design.md#io_uring-ring-ownership-and-migration).

Ending kernel access does not require destroying reusable state. A consumed scalar read returns its buffer and releases its permit, while its cached request can retain the reader's bound socket for reuse. Close and drop the bound reader before exclusive socket close.

Some output operations acknowledge issuance before completion so their caller can produce input needed for further progress. `submission_notifications.rs` tracks consumed SQ positions. Staging alone cannot acknowledge issuance, and a driver must not block waiting for completion while it owes such a receipt.

## Borrowed and owned socket paths

A successful borrowed write performs admission checks and a nonblocking send inside the application poll. It returns without creating wait machinery or visiting the reactor. On blocking, it retains a directional registration and waits; a notification is advisory, so the resumed operation retries its syscall. No independent kernel or worker payload access survives the poll.

Native epoll/kqueue registers a stable socket record and publishes directional readiness directly from native events. Readiness generations prevent a stale clear from losing a newer notification. The ordinary scalar reader may clear after a short read under the platform's EOF/OOB rules; conservative paths drain through `WouldBlock`. See [urgent-data policy](../design/socket-stream-oob.md).

The io_uring borrowed path uses direct nonblocking transfers plus a persistent directional `PollAdd` monitor when it must wait. Its monitor CQE publishes readiness; the payload syscall still happens on the subsequent application poll. An owned scalar read instead queues a `Recv` carrying stable owned storage. Its CQE supplies the transfer result. Request/completion storage is reused by bound scalar readers. Successful scalar owned writes also try a direct send. Completion I/O therefore need not mean a kernel submission for every write.

Here “owned read” specifically means `read_owned(buffer)`. The pooled chunk API `read(cancellation)` also returns an owned value, but currently acquires a pool slot and uses the borrowed readiness-driven receive path, releasing that slot while waiting. Ownership of the returned value does not by itself select completion I/O.

## An io_uring turn

`Reactor::turn` in `crates/iouring/src/io_uring_selector.rs` currently:

1. Processes at most 256 commands, alternating preference between local and remote queues. This prevents a continuous producer from hiding completions.
2. Collects a snapshot of available CQEs. It publishes the entire snapshot before returning to the owning executor. Waker callbacks can reenter routing; application futures are not polled inline by CQ collection.
3. Stages cancellations, payloads, persistent monitors, then the notification poll, subject to SQ capacity. Retention precedes pointer publication.
4. Decides whether to submit now, return to the executor, or combine submission with an interruptible wait. Existing staged SQEs require a submission attempt on the next nonblocking turn; transient kernel errors can still leave them unconsumed. Overflow and deferred kernel task work can force a kernel entry even without new payload.
5. On successful nonblocking submission, accounts for issuance receipts and collects the resulting CQ snapshot before returning. Submission itself can complete operations immediately.
6. After a blocking-capable submit/wait, collects its CQ snapshot before returning. Transient errors retain progress state for a later turn.

Work which may have woken tasks, pending issuance receipts, command overflow, remaining unstaged commands, an unavailable notification poll, a zero timeout, or a retained explicit notification prevents an ordinary blocking wait. Local explicit notification permits are consumed at the wait decision; eventfd notifications remain observable until their CQE is collected and the counter is read. Those are progress mechanisms, separate from payload completion.

New owned payloads whose entry preparation cannot wake tasks can also be staged on a zero-timeout turn and submitted with the next turn. The policy bounds deferral by reactor visits, not elapsed time or successful issuance despite kernel errors. The measured experiment excluding payloads did not justify its extra low-concurrency CPU cost, so this policy remains. See the research ledger for the evidence and its limits; no universal submission policy is adopted as a requirement.

## How the earlier changes fit

| Change | Work or delay removed | Boundary preserved |
| --- | --- | --- |
| Direct native registration pointers | Token translation, map lookup and routing lock | Retain records across event batches and deregistration |
| Native event records dispatched in place | Copying and normalization of each event | Initialize records before decoding; reclaim after batch |
| Successful-write fast path | Constructing cancellation/readiness waits before a successful send | Admission and cancellation checks; short-write results |
| Directional readiness generations | Redundant probes after drained ordinary reads | New notifications, sticky EOF and selected OOB policy |
| Worker-local ring routing and reusable requests | Remote queue transfers, interrupts and repeated setup | Explicit scheduler identity and original-ring retirement |
| Deferred kernel task work and combined submit/wait | Repeated kernel entries and split idle transitions | Taskrun/overflow servicing and interruptible waits |
| Collection after submit/wait | Holding already available results until another turn | Complete a finite CQ snapshot; poll tasks through executor |

Batching issuance and promptly publishing its results are complementary. The unresolved policy is how much useful work to admit between kernel and executor visits under mixed loads; there is no universal batch count established here.

## Research method and open questions

An experiment names the boundary it changes, predicts an observable counter or delay, and measures that separately from the uninstrumented end result. Keep failed variants and source identities. Compare fixed offered rates for latency; run unrestricted capacity separately. Report actual-send RTT and latency from scheduled arrival separately, with missed targets, drain time, errors, process CPU, CPU placement, and task topology. Saturated p99 values at different request rates cannot establish a latency improvement at equal load.

An interval matters to latency when the next useful stage could already have progressed. A queued receive may be staged before its next request arrives, overlapping peer or client scheduling time. That queue interval is not automatically on the next response's critical path. Correlate input availability, published results and response progress before attributing RTT to a boundary.

The current raw control performs one read and one static response write. It isolates runtime/socket work from HTTP parsing but does not establish behavior for streaming, backpressure, slow readers, mixed CPU work or many workers. Prior post-submit results improve median RTT substantially while equal-rate p99 remains unresolved. Possible next boundaries are stage-to-issue delay, CQ-to-runnable publication, and runnable-to-poll delay inside a supervisor. Measure their frequency and contribution before adding budgets or caches.

See [the research ledger](../benchmarks/echo/worker-reactor-research.md) for hypotheses, evidence, and decisions. Open questions include task/driver fairness, batch duration under mixed work, ring migration, overlapped full-duplex traffic, and tail latency across offered rates. None is answered by the hello-world capacity result alone.
