# Interrupt frequency investigation

The direct-ring interrupt was exercised frequently in follow-up probes of the regressing prototype. Lightweight source counters record about 0.973 successful eventfd writes and 0.973 notification CQEs per lifecycle echo roundtrip, overwhelmingly from the same worker thread. Its frequency in the original uninstrumented matrix was not counted. Signalling, notification SQE submission/CQE production, entering a wait, and actually blocking are distinct events; none should be substituted for another.

## Evidence already present in the original matrix

The original uninstrumented 300-sample matrix records server lifecycle voluntary context switches in `server_cpu.voluntary_switches`. For native owned I/O with one worker, one connection, and 64-byte messages:

| Repeat | Before switches | Prototype switches |
| --- | ---: | ---: |
| 1 | 75,139 | 862,176 |
| 2 | 198,726 | 887,460 |
| 3 | 152,922 | 795,199 |
| 4 | 111,054 | 787,906 |
| 5 | 49,055 | 804,701 |
| Median | 111,054 | 804,701 |

The roughly 7.2-fold increase accompanies median throughput falling from 236,431 to 201,327 round trips/second and lifecycle CPU falling from 3.37 to 2.44 seconds. These process totals include warmup and shutdown and sum all server threads. They support substantially more voluntary blocking in the prototype, but neither isolate eventfd-induced wakeups nor establish a single causal kernel mechanism.

## Kernel probes

The [runner](run.py) records two randomized repetitions of each frozen before, PollAdd-prototype, and direct-read executable, both with and without BPF tracing: 12 samples. Each uses one worker, one connection, 64-byte messages, one second of warmup, and three seconds of measurement; server CPU 0, fixed client CPUs 4–7. Tracer userspace runs on CPU 8. All runs hold the exclusive host lock; no builds or source edits occur.

BPF tracks notification-tagged submissions and completions, enter arguments, descriptor-specific read/write calls, and worker context switches. The notification tag is `u64::MAX`. A ring-context map allows completions emitted outside the server thread to be attributed to the observed ring. The generated `.bt` programs, raw map output, [samples](aiko.json), and [source/hash metadata and derived counts](summary.json) are retained.

For the prototype and direct-read variants, `/proc/PID/fd` identifies fd 5 as the interrupt eventfd. Lifecycle payload pairs below are successful 64-byte payload CQEs divided by two; each echo uses one receive and one send. These totals include warmup and the final boundary-crossing request, and are deliberately separate from client measurement-window request counts.

| Variant | Repeat | Lifecycle payload pairs | Eventfd writes returning 8 | Notification CQEs | CQEs / payload pair |
| --- | ---: | ---: | ---: | ---: | ---: |
| prototype | 2 | 904,531 | 1,533,422 | 118,072 | 13.05% |
| read | 2 | 819,585 | 803,620 | 801,635 | 97.81% |
| prototype | 1 | 922,339 | 1,428,766 | 97,532 | 10.57% |
| read | 1 | 797,605 | 781,337 | 780,406 | 97.84% |

The PollAdd notification SQEs have opcode 6 and return readiness mask 1; direct-read notification SQEs have opcode 22 and return 8 bytes. Submission and completion counts agree in each probe. The counters establish real notification CQE production in these runs, rather than merely writes to an unobserved event counter.

Tracing substantially perturbs the workload: before controls give 236,615–241,559 round trips/second versus 214,345–220,086 with probes; PollAdd controls give 199,663–207,982 versus 226,178–231,226 with probes; direct-read controls give 189,147–191,121 versus 200,732–203,986 with probes. The resulting phase changes make these frequencies unsuitable as estimates of the original matrix's uninstrumented frequency. In particular, their context-switch counts must not replace the original totals above. These are diagnostic runs, not new capacity results.

Bpftrace emits warnings about inferred address spaces, integer casts in descriptor map keys, and discarded `delete` results. The retained output contains those warnings; reported descriptors and return values are small nonnegative fd numbers and signed syscall results, respectively.

## Worker-local source counters

A separate [source-counter experiment](source-counters/analysis.md) adds thread-local `Cell<u64>` counters to the exact PollAdd prototype and later direct-read variant. Counters are printed only at thread teardown, without shared atomic updates. Three randomized repetitions of each counted build and its frozen control give 12 runs under the same one-worker, one-connection, 64-byte workload and host lock. The [patches](source-counters/patches), [runner](source-counters/run.py), [source/build hashes](source-counters/provenance.json), and [raw samples](source-counters/matched/aiko.json) are retained.

| Variant | Notification CQEs / lifecycle echo estimate | Successful local eventfd writes / lifecycle echo estimate | Successful remote eventfd writes per run | Counted / frozen median throughput change |
| --- | ---: | ---: | ---: | ---: |
| PollAdd | 0.9728–0.9729 | 0.9731–0.9731 | 2 | −0.69% |
| Direct read | 0.9783–0.9783 | 0.9784–0.9784 | 2 | +2.17% |

The echo estimate is untagged payload CQEs divided by two. In this workload those CQEs are original socket receive/send completions; the final EOF receive contributes half a pair. The counter also includes any unsuccessful attempts or partial transfers, so it is an estimate rather than an exact successful-request count. It covers the full server lifecycle, not just the client's measurement window. Local means the installed io_uring worker thread; with exactly one worker, this is also the target ring's owner. This classification must not be generalized to multiple worker/controller groups. All eventfd writes succeed, and no notification CQE reports an error.

For PollAdd, each run records 830,258–876,225 successful local writes and 830,010–876,021 notification CQEs, versus only two remote writes. Wait calls and voluntary context switches are both approximately one per lifecycle echo. A wait call can return without sleeping; these are separate observations, not interchangeable counts. PollAdd additionally incurs synchronous drain attempts, including `WouldBlock`; the direct-read variant consumes the event counter through its CQE.

These counters perturb median throughput much less than BPF, and the PollAdd voluntary-switch totals remain close to their frozen controls. Nevertheless, the large difference from the BPF notification rate shows that event-loop interleaving is timing-sensitive. They establish frequent interrupt activity dominated by same-worker signals; they do not recover an exact rate for the original uninstrumented matrix or prove that eliminating these signals alone removes the 14.8% regression.

## Relationship to io-event

The later direct-read variant follows io-event's core mechanism: a persistent eight-byte destination, one outstanding eventfd read across timeouts, a sentinel completion tag, and rearming before an idle wait. The 14.8% matrix regression was measured on the earlier PollAdd-plus-synchronous-drain prototype, rather than this later direct-read variant.

Wake policy differs. Ruby sets `backend.blocked` around the GVL-releasing wait and clears it before processing CQEs; `wakeup` writes only while that flag is set. Core's executor publishes its sleeping flag before `driver.turn(None)`, and that call can process completions. Publishing a completed task can therefore call the same worker's wake path, clearing its sleeping flag and invoking the ring notifier while the worker is already executing. This window applies when a CQE arrives before the initial completion snapshot in `turn(None)`. CQEs that wake an actual blocking wait are processed on a later turn, after the sleeping flag has been cleared. This is broader than signalling only a worker blocked in the kernel. The source counters confirm overwhelmingly same-worker signalling in this one-worker workload. They do not capture call stacks or establish the contribution of this path to the original throughput regression. A future fix should avoid local completion-publication interrupts while preserving the cross-thread publication/parking handshake; that change has not been implemented here.
