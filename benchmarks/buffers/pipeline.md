# Pooled receive, parsing, and zero-copy sends

This benchmark prototypes stable shared storage through an entire receive → parse → immutable subrange → send cycle. It changes no production scheduler interface. The executable uses raw io_uring so kernel registration and send costs can be measured independently of executor and scheduler bookkeeping.

## Results and recommendation

The [960 loopback samples](results/pipeline-loopback-summary.md), [360 physical-network samples](results/pipeline-network-summary.md), and [56 longer validation samples](results/pipeline-validation-summary.md) use one optimized executable, SHA256 `ff99a941d496568f8d30cdfcafeb811d0b9cb969ad788f46a43c49027e077ff1`. The [environment record](results/pipeline-environment.json) identifies both hosts and their network configuration. Raw JSONL files and machine-readable summaries sit beside those tables.

The longer validation uses seven randomized repeats per 64/128 KiB cell, one TCP connection, retained parser views, one ring, and 128 MiB of incoming request data per mode. Fixed-zero-copy/ordinary throughput ratios are medians of matched per-repeat ratios:

| Request chunk | Fixed zero-copy / ordinary throughput | Observed pair range | Fixed / unregistered zero-copy throughput |
| --- | ---: | ---: | ---: |
| 64 KiB | 1.004× | 0.998–1.017× | 1.002× |
| 128 KiB | 0.989× | 0.931–1.132× | 0.998× |

All 43,026 zero-copy notifications in the longer physical-network validation report no copying fallback. The broad network sweep has four fallback notifications out of 256,541; partial sends occur and are retried without changing response contents. Every one of the loopback sweep's 2,884,800 notifications reports copying fallback. For example, retained-view, local, single-connection 64 KiB TCP achieves median 4.978 GiB/s with ordinary sends and 4.151 GiB/s with fixed zero-copy sends. Loopback is therefore a useful fallback-overhead control, not evidence that a NIC zero-copy path is copying.

CPU measurements do not establish a repeatable saving. Some short-run cells initially suggested substantial reductions, but the longer validation reverses that impression: paired fixed-zero-copy/ordinary CPU ratios have medians 1.639× and 1.275× at 64/128 KiB, with ranges 0.259–3.437× and 0.151–3.695×. Even ordinary and registered-copy controls vary considerably despite using the same I/O opcodes. Keep those raw measurements; do not present a CPU-efficiency win or a measured explanation for the variation. A controlled CPU profile would be needed before attributing it.

These stop-and-wait network exchanges do not establish a general throughput improvement, a universal size threshold, or a benefit over Tokio. Cross-ring handoff works and is measured, but the channel/eventfd cost and the bonded physical network make its performance workload-dependent. The fixed-versus-unregistered-zero-copy validation differences are negligible here; registration sharing is nevertheless a useful storage/lifetime capability.

The design recommendation is to retain stable shared pool ownership, exclusive writable leases, immutable parser/send views, and separately retained per-ring registrations. The prototype demonstrates that these existing lease types can support registered zero-copy sends and cross-ring handoff without an application payload copy or a new public buffer abstraction. Keep copying sends as the current performance baseline. Integrate zero-copy as a backend choice only with proper production cancellation, retirement, and unsupported-kernel fallback; do not route every pooled send through zero-copy based on these results. Fixed registration and provided-buffer publication remain separate mechanisms.

## Ownership

The existing `BufferPool` supplies exclusive writable `BufferLease` values. A receive fills a lease with a predictable 16-byte length/sequence header followed by a payload. The parser validates the header, freezes the allocation, and sends an independently owned view of the payload. No application payload copy or per-message shared-owner allocation is necessary. The retained variant holds the previous request's header view until the next request has arrived and checks its sequence again, exercising storage retained beyond the send.

Ordinary sends retain their view through completion. Zero-copy sends retain it through the send result and any notification promised by `IORING_CQE_F_MORE`. A notification with `IORING_CQE_F_NOTIF` permits that kernel owner to be released; parser views may still retain the slot. Short sends create a view for the remaining range while independently retaining any kernel owner for the completed prefix. Receives continue while earlier zero-copy notifications remain outstanding.

Registration has a separate owner: each ring retains the storage independently of application leases. Fixed-buffer handoff clones the receive ring's table into the send ring. A completed receive can move to the sender without copying its bytes or unregistering and registering its allocation. The receive ring unregisters after its own requests retire; the sender keeps its registration and storage until its sends and notifications finish. A final capacity check confirms that every slot has returned.

Unwinding deliberately leaks any unretired request owners, preserving memory safety without adding an experimental asynchronous cleanup implementation. This is benchmark failure handling, not the proposed production cancellation/shutdown design. The prototype does not test cancellation or `mem::forget`, implement receive multishot/provided-buffer publication, or implement NIC zero-copy receive.

## Compared modes

- `copy`: ordinary `Recv` and `Send` with unregistered pool storage.
- `registered-copy`: the same operations and ownership with a registered table; registration alone changes no send opcode.
- `zc`: ordinary `Recv`, followed by unregistered `SendZc`.
- `fixed-zc`: ordinary `Recv`, followed by `SendZc` using a registered index and payload subrange.

All modes perform the same registration-index lookup before sending. Fixed registration is used for sending; this does not repeat the earlier `ReadFixed`/`WriteFixed` receive experiment. Registration setup is outside the throughput timer; final send retirement and explicit unregistration are inside it.

A local placement receives and sends on one ring and thread. Handoff receives on that ring and passes immutable views through a bounded channel to a second thread with its own ring. An eventfd interrupts the sender when work arrives or the channel closes. Both rings retain their own request slots. This explicitly measures cross-thread coordination, rather than representing free task migration in the production Native executor. Availability returns use the portable pool's existing atomic bitmap.

Pool capacity is `4 × connections + 1`; backing allocations include alignment padding. Full payload pages use the same normal-page/refault preparation as the fixed-buffer experiment. A run remains bounded instead of allocating more payload memory under pressure.

TCP receives use `MSG_WAITALL` for each known-length synthetic request, with short completions still handled. Each connection has one request/response exchange in progress; separate connections provide concurrency. UDP uses one datagram per request and response. This is predictable framing, not an HTTP parser, TLS, QUIC, or an application-level end-to-end zero-copy claim. Peers validate every returned payload byte, including sequence-dependent boundary bytes.

## Measurement

Aiko runs the same optimized executable for every mode. The loopback peers are threads in that executable; physical-network peers run the exact same executable on Hana over the wired network. Aiko negotiates 10 GbE; Hana uses an LACP bond of two 1 GbE ports, so a single flow is limited to one 1 GbE member and different connection tuples can use different members. Loopback CPU measurements therefore include client threads, while physical-network CPU measurements include only the server. All server threads share CPUs 2–5. CPU time is process user + system time, not a whole-system energy or CPU measurement; interrupt and other separately accounted kernel activity are not isolated. Timed samples hold `/tmp/core-scheduler-perf.lock` exclusively; no builds or validation runs are launched on Aiko during measurement.

The runner randomizes configuration groups and mode order with a fixed seed. Results compare matched per-repeat ratios, then take their median. Raw samples record user/system CPU time, enter and completion counts, short sends, capacity stalls, zero-copy notification usage, and executable SHA256. `IORING_SEND_ZC_REPORT_USAGE` distinguishes copying fallback from actual zero-copy. Reported byte rates count response payload bytes once; the transport also carries request bytes and framing.

The zero-copy usage flag is essential: loopback and NIC behavior can differ. A faster or slower `SendZc` opcode does not establish whether the payload avoided copying without checking those notifications.

## Reproduction

Build on Linux:

```sh
cargo build --release --locked -p core-scheduler-buffer-benchmark --bin pipeline
python3 benchmarks/buffers/pipeline.py --binary target/release/pipeline --output benchmarks/buffers/results/pipeline-loopback.jsonl
```

A single retained-view handoff run:

```sh
target/release/pipeline fixed-zc tcp 65536 8 1000 retained handoff
```

The runner also supports `--remote hana.local`, `--client-binary PATH`, and `--address SERVER_IP`. When the SSH credentials for both machines are on another workstation, use `--server-host aiko.local --server-binary REMOTE_PATH`; `--binary` then identifies a local copy of the measured Linux executable for hashing. The client and server executable must match.

Regenerate and validate either summary:

```sh
python3 benchmarks/buffers/summarize_pipeline.py benchmarks/buffers/results/pipeline-loopback.jsonl --json benchmarks/buffers/results/pipeline-loopback-summary.json --markdown benchmarks/buffers/results/pipeline-loopback-summary.md
```

## Kernel references

The [zero-copy send documentation](https://man7.org/linux/man-pages/man3/io_uring_prep_send_zc.3.html) defines fixed-buffer subranges, copy-usage reporting, and the distinction between send results and memory-reuse notifications. The [buffer-table cloning documentation](https://man7.org/linux/man-pages/man3/io_uring_clone_buffers.3.html) describes cross-ring registration sharing. The handoff prototype requires that cloning support instead of claiming compatibility with older kernels; production integration would retain ordinary-send fallbacks.

## Validation and source identity

Linux workspace/all-target/all-feature tests and measured region coverage pass: **29,823/29,823**, with eight proven `unreachable!()` regions excluded by the shared task. macOS workspace/all-target/all-feature coverage passes **21,789/21,789**, exercising the unsupported-platform entry point rather than Linux operations. Linux workspace Clippy and macOS benchmark Clippy pass. Tests cover ordinary and registered sends, both transports and placements, retained views, real short sends, truncated requests, connection resets, abandoned in-flight reads, pool pressure with pending completions and independent owners, table unregistration, and complete pool recycling.

The Linux coverage task uses Arch's matching LLVM tools explicitly. A temporary Cargo wrapper adds `--ignore-filename-regex '^/usr/src/debug/rust/'` to the coverage invocation, excluding the toolchain's remapped standard-library sources without excluding project files. The repository's shared Bake coverage gate then checks the generated report normally.

All measured samples use the executable hash recorded above, also verified on Hana. After measurement, additional tests were added and two impossible CQE-state panic guards were expressed with `unreachable!()`. Normal operation selection, pool access, retirement, counters, and timed loops are unchanged. The final release rebuild has SHA256 `21c9bf4e51695a1d918f09447b5170aba29a186b73ea4814600fde0d31666bf4`; it is explicitly distinct from the measured executable. The measured server artifact is preserved at `/home/samuel/.cache/core-scheduler-pipeline/pipeline-measured` on Aiko.
