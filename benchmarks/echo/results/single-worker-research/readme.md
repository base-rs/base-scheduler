# Single-worker research evidence

The [research report](../../single-worker-research.md) explains the results. No diagnostic production patch in this directory is applied to the workspace.

Run this from the checkout root to validate every sample matrix and the recorded script, patch, and executable identities, then regenerate the tables:

```sh
python3 benchmarks/echo/results/single-worker-research/summarize.py
```

The [generated summary](summary.json) preserves numerical precision and latency medians; the [tables](tables.md) include throughput ranges. Each aggregate JSON retains all samples, including client and server CPU usage. Throughput conclusions use only uninstrumented samples. Small startup/shutdown work remains in whole-process syscall and instruction counts.

| Directory | Evidence |
| --- | --- |
| [profile](profile/aiko.json) | 30 uninstrumented baseline controls, ten whole-process stat samples, and five cycles profiles. The `stat/` files preserve counters; `perf/` preserves self-cost reports. |
| [semantic-controls](semantic-controls/aiko.json) | 39 samples comparing ordinary/conservative Tokio reads, task-wrapper bypasses, both adapters, and Native's separate readiness selector. |
| [receive-errors](receive-errors/aiko.json) | Twelve separate stat samples; `sys_exit_recvfrom` is filtered for `ret == -11`. |
| [poll-first](poll-first/aiko.json) | 36 samples comparing the receive poll-first hint and its unchanged Tokio controls. |
| [deferred](deferred/aiko.json) | 54 samples comparing deferred single-issuer rings and unchanged Tokio controls after the executor changes. |
| [matched-policy](matched-policy/aiko.json) | 60 samples, five repetitions per cell, comparing ordinary raw Tokio, conservative polling raw Tokio, and both Tokio adapter buffer interfaces. |
| [scripts](scripts) | Exact build and measurement scripts used on Aiko, including the privileged-profiler supervisor. |

## Provenance

The baseline server hash is `1e58573bd54ab7e4339101298e31846f9ec26f9951080fe441f51e63c0d99e1e`, built at `9386f7c4f6cb0dccd7e528512476d2cf963f7a88`. The changes from that source to merged PR #17 (`130cd4a67e526e80ba9a5e7e7ebbe81494bd903b`) affect comments, contracts, release notes, and evidence; executable code is unchanged. The fixed client hash is `db6c914ab75011477ec6232e681046b1e75532cbaa1c81663bac984591675cf3`, from source `5736301fded46c4e5f11d5387950ae2ee0807b94`.

Each subsequent `build.json` records its new executable identity and source patch hash. `semantic-controls/change.patch` modifies only benchmark source. `poll-first/change.patch` and `deferred/change.patch` modify production source for rejected diagnostics and both additionally use the unchanged semantic-controls benchmark patch. `matched-policy/change.patch` contains all its benchmark-only modifications and no production changes.

Aiko runs Linux `7.2.6-arch2-1`, Rust `1.99.0` (`b940084d7eb6a299eb4bfeb8e34901bc051e7ac4`), LLVM `23.1.1`, and Cargo `1.99.0`. Builds use `cargo build --release --locked -p core-scheduler-echo-benchmark`, with default `opt-level=3`, no explicit LTO, and no target-CPU override. `profile/aiko.json` includes topology, governor settings, and perf version. The default powersave governor is unchanged. [Additional provenance](provenance.json) records the compiler and relevant environment overrides; the separate [setup trace](deferred/setup.strace) confirms successful deferred worker-ring setup without mixing tracing into timings.

Raw perf `.data`, callgraph reports, optimized binaries, targets, and duplicate per-sample JSON remain outside the repository in `~/.cache/core-scheduler-single-worker-research/` on Aiko. Self-cost text reports remain in this archive; symbol sample shares may include inlined callers and must not be read as isolated wrapper costs. Binaries in the cache are frozen before measurement.

## Reproduction

The scripts intentionally preserve the exact host paths used by this run. They are research harnesses, and their output directories must not already exist. Copy `scripts/` into `~/.cache/core-scheduler-single-worker-research/` on Aiko and prepare a clean detached checkout at `~/Developer/core-rs/core-scheduler-single-worker-research` using merged source `130cd4a`. Keep the fixed client and baseline server at the cache paths in `profile.py`, or adapt those paths and record new hashes.

`profile.py` generates the baseline controls, counters, and profiles. `build-controls.py` requires the clean checkout and adds the semantic-control benchmark modes. After it finishes, use `run.py` with its generated `controls-build.json`, and the 13 backend names recorded in `semantic-controls/aiko.json`, to reproduce that matrix. `receive-errors.py` uses the frozen controls executable and adds the receive-exit event filter to a copy of the helper.

`build-poll-first.py` applies the hint on top of the benchmark controls and freezes a new executable. `build-deferred.py` restores the production selector from HEAD before applying its independent flags, preserving the benchmark controls. `build-matched-policy.py` creates a separate worktree, applies the benchmark controls, adds the polling-readiness variant, and freezes its executable.

For example, the final matched-policy matrix uses:

```sh
research_cache="$HOME/.cache/core-scheduler-single-worker-research"
python3 "$research_cache/run.py" \
  --output "$research_cache/matched-policy-screen" \
  --build "$research_cache/matched-policy-build.json" \
  --helper "$research_cache/sample-runner.py" \
  --client "$HOME/.cache/core-scheduler-owner-wake-final/client" \
  --repeats 5 \
  --workload 1,32,64 --workload 1,32,4096 --workload 4,128,64 \
  --case "matched,tokio-policy-native,$research_cache/matched-policy" \
  --case "matched,tokio-policy-poll,$research_cache/matched-policy" \
  --case "matched,tokio-adapter,$research_cache/matched-policy" \
  --case "matched,tokio-adapter-poll,$research_cache/matched-policy"
```

For the other screens, `cases`, `workloads`, `repeats`, and executable identities in each aggregate JSON specify the corresponding `run.py` arguments. The runner uses deterministic shuffled order, one warmup second, three measurement seconds, server affinity CPU 0 through workers−1, and client CPUs 4–7. Builds, timings, and profiling hold `/tmp/core-scheduler-perf.lock` exclusively; serialize source mutations too when rebuilding variants in one checkout. Profiling needs passwordless `sudo -n perf`; the server itself runs as the original user.

No runtime behavior is changed by the retained research report. Verification consists of matrix and provenance checks plus reproducing the numerical summary; the diagnostic executables are not proposed implementations and have not undergone the production conformance/coverage workflow.
