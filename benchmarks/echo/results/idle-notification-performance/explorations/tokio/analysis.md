# Tokio follow-up performance experiments

Base: `a98f172f8b06f23b34a354a4b6f6c294aa5859ab` (merged PR #14).

These source-change screens use default Cargo release optimization without profile overrides. The separate [thin-LTO diagnostic](thin-lto-analysis.md) changes only that build setting. Fixed external client and disjoint server/client CPU pins are recorded in each `aiko.json`. Build/test operations take the shared `/tmp/core-scheduler-perf.lock`; all samples take it exclusively. Each sample has 1 second warmup and 3 seconds measurement, with randomized execution and 3 repeats.

## Initial waiter and future-state screen

`waiter-swap` replaces compare_exchange(false, true, Acquire, Relaxed) with swap(true, Acquire). `flat-state` combines that with caching the initial OnceCell registration reference and flattening one nested async block.

| Variant | Backend | Median req/s | Minimum | Maximum |
| --- | --- | ---: | ---: | ---: |
| baseline | tokio-adapter | 648,433 | 646,751 | 649,339 |
| swap | tokio-adapter | 648,195 | 643,778 | 649,228 |
| flat | tokio-adapter | 648,906 | 646,664 | 649,131 |
| baseline | tokio-adapter-poll | 641,386 | 641,372 | 647,609 |
| swap | tokio-adapter-poll | 649,634 | 646,099 | 649,675 |
| flat | tokio-adapter-poll | 653,183 | 650,362 | 656,103 |
| control | tokio | 695,044 | 693,933 | 699,650 |

## Cancellation inline screen

`cancellation-inline` restores baseline and adds `#[inline]` to Cancellation::check, is_cancelled, and cancelled. Baseline optimized disassembly retains indirect GOT calls to check()/is_cancelled() in hot Tokio/native task pollers. The inline build eliminates those two symbols/calls, while its Tokio poll bodies grow slightly.

| Variant | Backend | Median req/s | Minimum | Maximum |
| --- | --- | ---: | ---: | ---: |
| baseline | tokio-adapter | 648,114 | 646,271 | 649,183 |
| inline | tokio-adapter | 650,347 | 649,019 | 657,932 |
| baseline | tokio-adapter-poll | 653,272 | 648,271 | 655,450 |
| inline | tokio-adapter-poll | 647,994 | 642,149 | 648,655 |
| baseline | iouring-poll | 569,129 | 565,449 | 569,733 |
| inline | iouring-poll | 562,331 | 560,962 | 563,900 |
| control | tokio | 698,572 | 692,521 | 708,499 |

The waiter/state screen is flat for owned operations and weakly positive for borrowed operations; no retention decision is justified by that pilot alone. Cancellation inlining shows mixed/no throughput benefit and is rejected rather than broadened.

`provenance.json` records immutable binary paths/checksums, source patches, base commit, compiler and build settings. Individual JSON samples and runner/client fingerprints remain in the two run directories.

## Small syscall/admission wrapper inline screen

`wrappers-inline` restores baseline then adds `#[inline]` to TokioScheduler::check_open/check_socket and socket_io::recv/send/recv_poll/send_poll. All six symbols disappear from the optimized binary as intended. No public semantics change.

| Variant | Backend | Median req/s | Minimum | Maximum |
| --- | --- | ---: | ---: | ---: |
| baseline | tokio-adapter | 650,932 | 650,424 | 653,853 |
| inline | tokio-adapter | 643,558 | 635,744 | 645,638 |
| baseline | tokio-adapter-poll | 652,592 | 648,581 | 656,445 |
| inline | tokio-adapter-poll | 643,811 | 639,161 | 647,977 |
| control | tokio | 700,579 | 691,701 | 702,435 |

The wrappers pilot does not improve capacity. Rejected; no broader matrix or source retention justified.

## Retention decision

No implementation changes are retained from these follow-up pilots. The weak borrowed-only gain from waiter/state variants is insufficient to justify additional bookkeeping changes while owned capacity is flat. Cancellation and small wrapper inlining both demonstrably remove calls but do not produce a robust measured gain. This archive preserves timing samples, patches, and binary hashes; assembly remains in the remote experiment cache.
