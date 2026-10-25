# Portable buffer policy

This is a historical experiment. Worker-provided pool construction and its receive-ahead integration have been removed from ordinary readers. Reproduce the comparisons at the recorded source revisions; current applications overlap reads and writes explicitly.

The portable interface now has one `BufferPooling::buffer_pool(options)` factory and an opaque backend receiver. Applications choose their payload budget, size, and alignment; the backend chooses allocation, reuse, registration, and receive strategy. Ordinary pools keep an exact shared total capacity independent of worker count. The locality experiment remains available only through the concrete `IoUringSelector::buffer_pool_per_worker(options)` method, accessed from Native through `scheduler.selector()`.

This refactor preserves the existing monitor and provided-buffer receive algorithms. It adds no payload allocation or per-message policy lookup. The [worker-owned report](worker-owned.md) records the earlier implementation and strategy comparisons; this check isolates the interface refactor rather than comparing those strategies again.

## Optimized comparison

The [112 raw samples](results/portable-policy.jsonl) contain seven randomized matched repetitions of eight cases on Aiko's Ryzen 9 9950X3D, Linux 7.2.6-arch2-1, and Rust 1.99.0. Both binaries use Cargo's release profile with debug information (`CARGO_PROFILE_RELEASE_DEBUG=1`). No builds, tests, or other benchmark runs overlap timing; the whole run holds `/tmp/core-scheduler-perf.lock`.

Every case has eight connections, eight-message bursts, 1,000 warmup bursts, and 10,000 measured bursts per connection. Affinity is CPUs 2–15, without individual thread pinning. The total payload capacity is 64 slots for both layouts: `total` uses an ordinary shared pool with monitor receives; `worker-equal` divides that capacity equally among the workers' provided-buffer rings. Each sample verifies the received byte count and pool capacity. These are loopback receive workloads without parsing, TLS, QUIC, or physical-network traffic.

Baseline source is `5e745a22f10f1e5f084c813f8454daf1be1a4d2a`; the candidate is the original pre-rebase portable-interface refactor at `facf19696a8c8a69154a28a3b8fb8ed9583284e9`. Later edits before that commit changed documentation and experiment artifacts, without changing executable Rust behavior. Both measured binaries preserve the baseline's task admission. The branch was subsequently rebased onto `main`, inheriting the separately measured [round-robin admission change](../echo/task-admission.md); these historical samples do not measure the rebased branch. Binary SHA-256 values are recorded in every row:

- Baseline: `0cf667b38b5cb004014ef4396b39e5235500078abc4fdee7c105015026c81246`.
- Candidate: `fb209685b7a66cc7cb55c5036cf73661c5bcdc09df1d4b737332894305e4a638`.

Changes in messages per second, candidate versus baseline. Medians and ranges use per-repeat matched ratios:

| Transport / payload | Workers | Pool layout | Median change | Repeat range |
| --- | ---: | --- | ---: | ---: |
| TCP / 64 KiB | 1 | Shared monitor | −0.9% | −7.0% to +3.5% |
| TCP / 64 KiB | 1 | Worker provided | +0.6% | −2.7% to +1.6% |
| TCP / 64 KiB | 4 | Shared monitor | +0.3% | −10.2% to +16.7% |
| TCP / 64 KiB | 4 | Worker provided | −0.9% | −4.1% to +29.3% |
| UDP / 8 KiB | 1 | Shared monitor | −1.1% | −1.7% to +1.3% |
| UDP / 8 KiB | 1 | Worker provided | +0.2% | −2.5% to +1.4% |
| UDP / 8 KiB | 4 | Shared monitor | −1.0% | −7.2% to +0.2% |
| UDP / 8 KiB | 4 | Worker provided | +4.8% | −13.0% to +17.0% |

The geometric mean across the eight case medians is +0.2%. There is no clear aggregate change; this is neither a speedup claim nor proof of equivalence. Multiworker variation remains substantial, and the shared-pool UDP medians are about 1% lower. This short check does not isolate such small changes from placement, build layout, or run-to-run effects, and does not change the recommendation to retain ordinary monitor receives as the portable default.

## Reproduce

Build each revision separately using `CARGO_PROFILE_RELEASE_DEBUG=1 cargo build --release --locked -p core-scheduler-buffer-benchmark --bin sockets` and preserve both executables. The runner creates `samples.jsonl` exclusively, refusing to append to an existing run:

```sh
mkdir /tmp/portable-policy-results
flock /tmp/core-scheduler-perf.lock env PERF_LOCK_HELD=1 \
  python3 benchmarks/buffers/experiments/portable-policy/run.py \
  /tmp/portable-policy-results /path/to/sockets-baseline /path/to/sockets-candidate
```
