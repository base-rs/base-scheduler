# Worker/reactor experiment evidence

Read the [research ledger](../../worker-reactor-research.md) for the hypothesis,
decision and limits. The immediate-payload-submission prototype is rejected;
the runtime submission predicate is unchanged. This directory retains the
measurements and patches used to reach that decision.

Baseline is main `5cc179985803e1da812353df73868db668234202`. Hana runs one worker
and one supervisor containing 64 connection futures. The raw base-http control
performs one arbitrary read and one static 78-byte response write using reused
16 KiB buffers. The client validates response bytes. This is not HTTP framing or
multi-worker scaling evidence.

- `fixed-240k/` records 20 samples: five interleaved randomized rounds of owned
  baseline/prototype, borrowed control and Tokio control at 240,000 offered
  requests/s. Each has two warmup and eight measured seconds. Scheduled latency
  includes lateness behind previous requests; RTT starts at actual send.
- `unpaced/` records 40 separate capacity samples: five rounds at one and 64
  connections, with one warmup and four measured seconds. Process CPU is sampled
  over the measured capacity interval. Fixed-client whole-run CPU fields include
  warmup and must not be substituted for that interval.
- `analysis.json` preserves medians and within-round ratios. Run `summarize.py`
  to regenerate it from the retained samples and unpaced summary.
- `diagnostic-*.stderr` retains cumulative prefixes. `diagnostic-analysis.json`
  preserves raw last-prefix counters and derived distributions. Run
  `summarize-diagnostics.py` to regenerate it. Diagnostic client JSON files record
  byte validation for the three strided baseline/prototype runs, borrowed
  controls and three pseudorandom baseline sampling controls. These timings do
  not belong to the uninstrumented cohort.
- `candidate-policy.patch` applies the rejected four-line classification change
  to the baseline. `diagnostic-baseline.patch` and `diagnostic-candidate.patch`
  each apply directly to that baseline, including sampled read-state probes.
  `diagnostic-random.patch` applies after `diagnostic-baseline.patch` to replace
  strided sampling with high bits of a deterministic 64-bit LCG.
- `binaries.sha256` identifies the frozen binaries and wrk. The complete local
  artifact bundle retains gzip-compressed baseline, prototype and fixed client;
  `artifact-verification.json` records matching decompressed hashes and the
  archived baseline source identity. The full diagnostic binaries remain on
  Hana, with their source patches retained here.
- `coverage-final.log` covers the retained runtime policy and bounded-read test
  plus lifecycle-fixture corrections: 7,552/7,552 measured source regions.
  `coverage-candidate.log` separately covers the rejected prototype:
  7,558/7,558. Clippy passes. The original Arch compiler-source mapping workaround
  is recorded in `verification.sh`; it excludes no project source.

Server threads use CPU 0, controller CPU 1 and client threads CPUs 2, 3, 6 and 7,
leaving SMT sibling 4 unused. Environment files retain OS/CPU/compiler metadata,
source/binary paths, hashes and load snapshots. Optimized builds use the existing
hello benchmark release profile, without new compiler tuning. No builds, tests
or profiling run concurrently with uninstrumented timing.

The full artifact bundle is in the companion workspace at
`base-http/benchmarks/hello/results/execution-model-20261028/`; the remote
experiment is `/tmp/base-http-execution-model-20261028` on Hana. It additionally
contains tracked scheduler and HTTP source archives, build and regression logs,
the failing immediate-issuance prototype regression, raw wrk/client output,
setup/build/archive scripts and frozen uninstrumented binaries. Runner scripts
here preserve the original artifact layout; `http-run.py` needs the archived
HTTP harness and wrk in that layout. The summary scripts can run directly here.

The first prototype suite exposed two batching-dependent lifecycle fixtures.
The retained corrections inject a supported submission interruption before
asserting that a request remains staged. They preserve original-operation
retirement expectations. Full workspace validation on Hana is not claimed;
the focused Linux io_uring Bake suite and its conformance tests pass.
