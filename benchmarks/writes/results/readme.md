# Ordered stream writer research

The [follow-up optimization report](improvements/readme.md) records newer cancellation and completion-delivery changes, including the reduction in explicit ring entries. The measurements below describe the original prototype.

The [fresh Tokio parity controls](../../echo/parity.md) subsequently withdraw the global ordinary-delivery change because it regresses concurrent monitor traffic. Cancellation allocation improvements remain, with cooperative worker rings restored.

The prototype separates accepting an owned stream write from waiting for all its bytes. It preserves ordering, handles partial transfers, and releases completed buffer owners without requiring another application call. Immediate full sends remain direct, allocation-free syscalls. This is an optional capability for applications that can prepare useful work while output is pending; ordinary borrowed monitor writes remain the simpler choice when that overlap is unnecessary.

## Interface and execution

```rust,ignore
let mut writer = scheduler.socket_stream_writer(&socket)?;
writer.submit(&scheduler, header, &cancellation)
    .await.map_err(|(error, _buffer)| error)?;

// The preceding output can progress while this work executes.
let body = prepare_body();
writer.submit(&scheduler, body, &cancellation)
    .await.map_err(|(error, _buffer)| error)?;
writer.flush(&scheduler, &cancellation).await?;
writer.close(&scheduler).await?;
```

Production code must also close the writer after an error; the straight-line example only illustrates the successful sequence. Header and body have the same owned buffer type, which can be a reusable pool lease.

A writer admits one unfinished logical write. The next `submit` completes every byte of its predecessor before accepting the new owner. `flush` waits for accepted output; it does not acknowledge receipt by the peer. `close` stops admission, cancels unfinished output and waits for memory retirement, without implicitly flushing or shutting down the socket. Failed admission returns the new buffer unchanged. Errors after acceptance are terminal and retained for the next submission or flush. Independent socket aliases and independent writers are outside this ordering.

The submission token governs an accepted write through its send result. Cancelling a later flush cancels that wait only. Dropping the writer requests cancellation, while backend owners preserve all memory still accessible to the kernel. Forgetting an object can leak resources but cannot invalidate in-flight memory. Explicit scheduler arguments and compatibility checks remain part of every operation.

The io_uring writer first attempts a nonblocking direct send. If bytes remain, it transfers the owner to a reusable driver request and waits until the first SQE is consumed by the kernel, or the accepted operation has already terminated. It then lets application work continue. Queue-consumption receipts distinguish staging an SQE from issuing it and account for unrelated entries, partial submission and refill. This implementation relies on exclusively owned rings without SQPOLL. Ring sends use `MSG_WAITALL`, allowing the kernel to retry partial stream transfers internally; explicit partial-write and `EAGAIN` fallback paths remain. Driver completion processing advances and retires requests without spawning an executor task per write.

Tokio and Direct use the same direct-send fast path and spawn a task only for unfinished output. Their successful submission reports task admission, rather than the stronger io_uring kernel-consumption boundary. Native forwards the selector's writer capability. The prototype deliberately does not add a writer to Native's readiness selector, which has no executor, and does not implement vectored output, file output or production zero-copy sends.

## Measurement method

All measurements use optimized release builds on Aiko, with a frozen client, disjoint client/server CPU affinity, randomized cases and the shared performance lock. Raw files retain source revisions, binary SHA-256 hashes and each sample. Every variant uses the same two-buffer pool per connection, computes the same deterministic bytes and validates the complete response, including chunk order and alternating request patterns. No payload buffers are allocated inside the measured loop. These are loopback transport experiments, without HTTP parsing or TLS.

The default send buffer and a requested 64 KiB send buffer are separate cases. Linux reports an effective 128 KiB for the latter. Combining it with 256 KiB chunks deliberately creates backpressure. An earlier requested 4 KiB smoke test produced delayed-ACK stalls of approximately 100–200 ms; it is excluded from capacity conclusions.

Synthetic payload generation initially inlined separately into each backend future. Adding benchmark control code changed rankings despite unchanged writer implementations. Final comparisons therefore call one non-inlined generation helper shared by every backend. This reduces a code-generation confound, but does not establish that different executables have identical surrounding machine code or runtime conditions.

## Results

The five-repetition matrix below reports median complete responses per second. Each response contains all eight chunks except the 64-byte case. `work=8` adds eight dependency-chain iterations per cache line; it is a synthetic transformation, not a parser. Native columns all use io_uring. The three writer revisions are separate executables, so differences include any effects of changing the surrounding binary.

| Workers / connections; response | Raw Tokio | Tokio monitor | Tokio writer | Native monitor | Native deferred writer | Native issued writer | Native issued + `WAITALL` |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 / 1; 64 B, work=0, default send buffer | 273,640 | 270,887 | 271,949 | 273,536 | 272,536 | 269,082 | 270,576 |
| 1 / 1; 64 KiB × 8, work=8, default | 8,768 | 8,764 | 8,801 | 8,798 | 8,770 | 8,273 | 7,372 |
| 1 / 1; 256 KiB × 8, work=0, constrained | 1,848 | 1,852 | 1,850 | 1,859 | 1,896 | 2,276 | 2,275 |
| 1 / 1; 256 KiB × 8, work=8, constrained | 2,134 | 1,832 | 2,148 | 2,129 | 2,149 | 2,177 | 2,054 |
| 4 / 32; 256 KiB × 8, work=8, constrained | 8,397 | 7,964 | 8,719 | 8,174 | 9,519 | 7,939 | 7,906 |

The exact final implementation, `78846c3`, was then checked with all five modes from the same executable and three repetitions per cell. These medians supersede attributing performance to a revision based only on the earlier cross-binary comparison.

| Workers / connections; response | Raw Tokio | Tokio monitor | Tokio writer | Native monitor | Native writer |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 / 1; 64 B, work=0, default | 275,866 | 270,048 | 271,474 | 275,530 | 271,281 |
| 1 / 1; 64 KiB × 8, work=8, default | 7,479 | 8,840 | 8,779 | 8,225 | 8,857 |
| 1 / 1; 256 KiB × 8, work=0, constrained | 2,208 | 2,053 | 1,849 | 1,942 | 2,249 |
| 4 / 32; 256 KiB × 8, work=8, constrained | 8,022 | 9,485 | 9,235 | 8,116 | 8,049 |

The useful conclusions are narrower than either table alone suggests:

- Immediate 64-byte output remains close: the final Native writer is 1.5% below the Native monitor and 1.7% below raw Tokio. There is no asynchronous overlap to exploit in this case.
- Issued Native writes help the single-connection backpressure case with no extra generation work. The earlier matrix measured approximately 22% above the monitor; the final executable measured approximately 16% above it and 2% above raw Tokio. However, final writer samples span 1,900–2,249 responses/s and raw Tokio spans 1,840–2,230. Those overlapping ranges prevent a universal advantage claim.
- The final four-worker Native writer is 0.8% below its monitor and 0.3% above raw Tokio. Tokio's monitor is faster in this particular case. The deferred writer's earlier 9,519 responses/s does not persist with immediate issuance: it is not evidence that the final writer universally improves throughput.
- The apparent 64 KiB regression in the earlier issued binaries is not isolated to the writer. Unchanged raw Tokio modes in those same executables range from about 7,322 to 8,760 responses/s, and unchanged Native monitor modes also change. The final Native writer instead spans 8,662–8,858. It would be incorrect to attribute the earlier 16% median loss solely to submission receipts or `MSG_WAITALL`; the residual source/binary/runtime confound remains unresolved.

Initial instrumented runs clarify which mechanism is active. In those runs, the default-send-buffer 64 KiB case uses eight direct sends and zero ring `Send` SQEs per response. That suggests the later regression needs investigation beyond ring-write issuance, but does not establish the mechanism in every later binary. Under constrained-buffer 256 KiB output, the initial deferred writer used approximately 19 ring `Send` SQEs and 30 ring entries per response, versus approximately 40 direct sends and 13 ring entries for the monitor. This motivated kernel-managed partial-send retry with `MSG_WAITALL`; reducing application waiting alone is insufficient if it increases kernel entries and completion traffic.

Eight final instrumented samples at `78846c3` confirm the intended mechanism. The following values are means of two per-response counter ratios, with zero warmup and three measured seconds per sample. Counts include small startup and final-boundary contributions.

| Output path, work=8 | Direct send calls / response | Ring `Send` SQEs / response | Ring entries / response |
| --- | ---: | ---: | ---: |
| Native monitor, 64 KiB × 8, default | 8.00 | 0.00 | 1.00 |
| Native writer, 64 KiB × 8, default | 8.00 | 0.00 | 1.00 |
| Native monitor, 256 KiB × 8, constrained | 39.97 | 0.00 | 13.01 |
| Native writer, 256 KiB × 8, constrained | 8.00 | 8.00 | 20.27 |

With `MSG_WAITALL`, the constrained writer is now near one ring send per chunk, rather than the initial approximately 19 ring sends per response. It still pays more ring entries than the monitor, despite far fewer direct send calls. The default-buffer case still performs no ring sends, and the final monitor and writer instruction counts are nearly identical there. These counters support preserving direct sends and kernel-managed short-write retries; they do not turn the capacity results into a general writer advantage.

Keep the interface for applications that need ordered ownership transfer and useful overlap. Retain the direct-send fast path, request reuse and kernel issuance acknowledgement. These results do not justify recommending the writer for every stream write or claiming a general throughput win over monitors or Tokio. Future comparisons should use application work less sensitive to synthetic code generation, and confirm both single-connection and many-connection behavior before changing the default recommendation.

## Ordinary echo regression control

The submission receipt hook also touches the shared driver used by ordinary owned I/O. A separate 24-sample echo comparison checks that path between the deferred prototype (`49c8e5c`) and final implementation (`78846c3`). It uses 64-byte messages, one server worker, three repetitions, the same frozen echo client and disjoint CPU affinity. These are ordinary owned receive/send operations, not the new writer API.

| Connections | Native before | Native final | Raw Tokio before | Raw Tokio final |
| --- | ---: | ---: | ---: | ---: |
| 1 | 231,066 | 237,378 | 274,608 | 273,949 |
| 32 | 629,792 | 627,419 | 715,964 | 723,259 |

Native medians move +2.7% and −0.4%, respectively, with overlapping sample ranges. This short control finds no clear regression from the general receipt hook; it does not establish a speedup. Its ordinary owned Native path remains slower than raw Tokio, consistent with the distinction between completion-based receives and the persistent borrowed monitor path.

## Verification

The complete macOS Bake gate covers 23,918/23,918 measured regions at `78846c3`, with three invariant unreachable regions excluded. The complete Linux workspace gate covers 34,527/34,527 measured regions, with twelve documented unreachable regions excluded. Linux validation uses `684e25d`, cherry-picked as `4d6764a`: it differs from the measured implementation only by making the benchmark's already-validated backend fallback explicitly unreachable on Linux. It does not change reachable behavior.

Formatting, workspace Clippy across all targets and features, and workspace Rust documentation with warnings denied pass. Rust 1.85.1 checks pass for the root/Native/Direct/Tokio packages on macOS and the io_uring Linux target including tests. An independent reviewer approved the runtime implementation, benchmark invariant and final report. This prototype was exercised on Linux and macOS; no Windows execution results are claimed. [Verification metadata](verification.json) records the exact scope.

## Evidence and reproducibility

- `screen.json` is the initial 75-sample screen with inlined generation; it is retained as preliminary evidence, not the final performance conclusion.
- `alternating.json` forces monitor variants to alternate between both pool slots without overlapping I/O. It distinguishes buffer reuse from asynchronous dispatch.
- `counters.json` and `counters/` contain the initial 12 instrumented samples. Instrumentation changes scheduling; their throughput is not a capacity estimate.
- `final-counters.json` and `final-counters/` contain eight final instrumented samples and their raw perf CSVs. Their measurements use the same runner and frozen client as the capacity comparison.
- `shared-prepare.json` contains the completed 175-sample, five-repetition comparison using shared generation. It resumed only missing samples after an interrupted runner, with unchanged binary hashes. `resume-shared-prepare.py` records that continuation.
- `builds.json` maps the deferred, issued and issued-with-`MSG_WAITALL` binaries to their exact source revisions and hashes. The yield-only variant was built but excluded from the final matrix because yielding alone does not guarantee kernel issuance.
- `issued.patch` and `waitall.patch` retain each io_uring variant's source changes relative to the deferred shared-generation source.
- `final-controls.py` runs the final source with unchanged-mode controls from the earlier binaries.
- `final-controls.json` contains all 78 final and unchanged-mode control samples, with three repetitions per cell and final source/binary identities.
- `echo-control.py` compares ordinary owned echo operations between the deferred prototype and final source, independently of the new writer.
- `echo-control.json` retains all 24 echo control samples and executable hashes.

Use [the benchmark runner](../run.py) for new measurements. Each source variant needs its own Cargo target directory; sharing one target between worktrees previously produced stale executables, which were detected and rebuilt before the retained final comparisons. The frozen executables remain under Aiko's `/home/samuel/.cache/core-scheduler-ordered-writer-benchmark/`.
