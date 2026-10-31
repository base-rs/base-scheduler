# INVALID: ORIGINAL CANDIDATE BUSY-SPINS WHEN IDLE

# Invalid performance candidate

The original benchmark binary (19e11b1b8e9309fcf6f04117fd62294e8f309d7fd0abe8d620595bccde8c18b5) has an idle busy loop: a notification CQE rearms PollAdd while eventfd remains readable. The resulting throughput measurements do not establish an acceptable performance improvement. Preserve the original source.patch, benchmark, control, matched/, matrix/, syscalls/, and idle.json as rejected experimental evidence. Corrected production source and results will be stored separately at /home/samuel/.cache/core-scheduler-direct-notification-production.

# Direct ring wait and local notifications experiment

Source: clean 63da755 control, candidate is 63da755 plus source.patch. Separate worktree /home/samuel/Developer/core-rs/core-scheduler-perf-tokio, branch perf/native-wake-analysis. No native executor changes.

Build: cargo build --release --locked -p core-scheduler-echo-benchmark; CARGO_TARGET_DIR=/home/samuel/.cache/core-scheduler-perf-tokio; no profile overrides. Shared build lock and exclusive timing lock /tmp/core-scheduler-perf.lock. Candidate and control frozen in this result directory. Client is fixed original baseline binary db6c914ab75011477ec6232e681046b1e75532cbaa1c81663bac984591675cf3.

The candidate replaces native ring epoll waiting with io_uring submit_and_wait(1), or submit_with_args with an explicit finite timeout. Cross-thread notifications use eventfd and a single-shot PollAdd, rearmed only after its CQE. Same-worker notifications set a TLS route-local Cell<bool>, consumed at the old notification-drain boundary after productive and zero-timeout returns. Both in-memory and external notifications are drained together. No user buffer ownership or cancellation logic changes.

## Matched serial owned completion result

Three randomized repeats, backend iouring, 1 worker pinned CPU0, 1 connection, 64 bytes; client CPUs4-7, one second warmup and three seconds measurement.

- before: requests/sec \[230396, 238292, 233745], median 233,745; voluntary switches \[154313, 106041, 143216].
- after: requests/sec \[255608, 255564, 258359], median 255,608; voluntary switches \[11, 11, 11].

Candidate median throughput delta: +9.35%.

Raw evidence: matched/aiko.json and run-matched.log. Hashes are included in JSON. Initial iouring-poll screen against the older frozen selected binary showed 222,107 versus 184,935 req/s (+20.10%), but the matched owned-completion screen above is the main result.

Conformance: cargo test --locked -p core-scheduler-iouring --test conformance: 180 passed. Full package unit tests have not been adapted: several existing tests reference Reactor.events and epoll-specific failure injection points that this prototype removes.

One separate strace summary sample confirms 80,745 io_uring_enter calls, 7 epoll_ctl calls total (fallback/startup), 4 write calls total and 17 read calls total over the whole server lifecycle. Raw syscalls/strace-iouring-w1-c1-b64-r1.strace and syscalls/aiko.json. Instrumented throughput is not comparable to ordinary timings.

Remaining before adoption: adapt epoll-specific unit/failure tests and documentation, broaden matched cases (4KiB, concurrency32, workers4), verify fair CPU/latency comparison, and compare against pure Tokio. Finite timeout paths and notification rearming need targeted tests. Source intentionally remains a prototype; no commits/pushes or canonical repo edits.
