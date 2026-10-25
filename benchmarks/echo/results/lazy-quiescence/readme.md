# Notify runnable quiescence only when requested

Native previously notified its task-drain event whenever the outstanding runnable count reached zero, even when no drain had ever been requested. The event's notification path includes synchronization with its listener list. This change arms those notifications lazily, removing unused work from ordinary execution.

The notification flag shares the runnable counter's atomic modification order. A waiter registers its listener before setting the flag; either it subsequently observes zero, or the final retirement observes the flag and notifies the registered listener. All count observations mask the reserved bit. Arming persists for the rest of that task service's lifetime, avoiding deregistration races between concurrent or abandoned waiters. Task, worker, shutdown, and stealing contracts are preserved.

## Measurements

The [confirmation](confirmation.json), [wider screen](wide.json), [build manifest](build-manifest.json), and [source patch](source.patch) preserve the frozen comparison. Run `python3 benchmarks/echo/results/lazy-quiescence/summarize.py` to validate executable identities, complete repetition matrices, the common client, and the source patch hash, then regenerate [summary.json](summary.json).

The five-run confirmation improves owned serial throughput by 2.68%, from 256,609 to 263,498 echoes/s. Its median P99 changes from 5.683 to 5.443 microseconds. The owned baseline's rate range is 247,227–262,117; the candidate's is 258,197–267,892. Their overlap and variation matter when interpreting this small improvement. Borrowed serial throughput changes by +0.30%. Busy owned throughput changes by −0.53%, and busy borrowed throughput by +0.34%; this change does not resolve the busy-case gap.

| One-worker confirmation | Owned baseline | Owned candidate | Tokio | Owned candidate P99 | Tokio P99 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 connection, 64 bytes | 256,609 | 263,498 | 273,293 | 5.443 µs | 4.331 µs |
| 32 connections, 64 bytes | 645,983 | 642,574 | 701,639 | 53.919 µs | 53.375 µs |

The candidate remains 3.58% below ordinary Tokio in the serial confirmation and 8.42% below it with 32 connections. These are throughput ratios within this screen, not a complete attribution of the remaining difference. Owned Native still submits actual io\_uring receives; borrowed Native retains its conservative readable-until-`WouldBlock` policy, while ordinary Tokio may clear readiness after a short successful stream read. User buffer ownership, cancellation, panic containment, and cleanup are unchanged.

The wider screen has owned throughput changes from −1.27% to +1.40%, and borrowed changes from −1.38% to +0.27%. These small, mixed differences do not establish a general throughput gain. P99 changes are retained below and every run range remains in the generated summary. The selected benefit is avoiding unused quiescence synchronization, with the clearest measured improvement in small serial owned reads.

| Workers/connections/bytes | Owned change | Owned P99, baseline → candidate | Borrowed change | Borrowed P99, baseline → candidate |
| --- | ---: | ---: | ---: | ---: |
| 1/1/4096 | +1.40% | 5.443 → 5.431 µs | -1.38% | 5.323 → 5.343 µs |
| 1/32/4096 | -1.27% | 59.391 → 60.063 µs | -0.51% | 61.375 → 61.599 µs |
| 4/1/64 | +0.89% | 5.631 → 5.003 µs | -0.18% | 4.603 → 4.603 µs |
| 4/1/4096 | -0.03% | 5.911 → 5.651 µs | -0.59% | 5.463 → 5.463 µs |
| 4/32/64 | -0.07% | 56.895 → 57.343 µs | -0.07% | 55.071 → 54.463 µs |
| 4/32/4096 | -0.39% | 65.343 → 65.311 µs | +0.27% | 63.007 → 62.911 µs |

## Method and reproduction

The source comparison starts at merged `05548997beabb48b05d350362b0db4b03468c511`; commit `9b88c59dd6d7649a164505ed4131c7bbb0373ebf` contains the measured change. Later commits add documentation and evidence. The recorded candidate executable was built by applying the archived patch to the merged base, then restoring the disposable checkout. Both executables use the same locked dependencies, opt-level 3, debug information, 16 codegen units, no LTO, no incremental compilation, and no extra Rust flags. Frozen executables and build targets remain outside the repository.

Aiko runs a Ryzen 9 9950X3D, Linux 7.2.8, and Rust 1.99.0 / LLVM 23.1.1. Server CPUs are 0 through workers minus one; the fixed four-worker validating client uses separate physical cores 4–7 in the same L3 group. The governors remain `powersave`. All Aiko builds, tests, and measurements serialize on `/tmp/core-scheduler-perf.lock`.

This is persistent IPv4 loopback TCP echo with `TCP_NODELAY`, one outstanding request per connection, initialized reused 64 KiB server buffers, complete partial writes, and validated responses. Every capacity sample has one second of warmup and three measured seconds. Rates and P99s are medians of per-run values, not pooled latency histograms. The confirmation contains five shuffled repetitions of two cases and five variants, totaling 50 samples. The wider screen contains three shuffled repetitions of the six remaining combinations of one/four workers, one/32 connections, and 64/4096-byte payloads, totaling 90 samples. Separate screens retain their own controls; their medians are not combined. Three repetitions on one host do not establish sub-percent gains or production tail latency.

Build frozen baseline and candidate executables in disposable checkouts at the recorded revisions using `cargo build --release --locked -p core-scheduler-echo-benchmark` and the manifest's environment. Retain one Cargo target directory for both builds and copy each executable before building the next. Use the existing [measurement runner](../worker-completions/measure.py), supplying `BASELINE`, `CANDIDATE`, the recorded validating `CLIENT`, and a fresh output directory:

```sh
python3 benchmarks/echo/results/worker-completions/measure.py \
  --variant owned-baseline=iouring="$BASELINE" \
  --variant owned-lazy=iouring="$CANDIDATE" \
  --variant borrowed-baseline=iouring-monitor="$BASELINE" \
  --variant borrowed-lazy=iouring-monitor="$CANDIDATE" \
  --variant tokio=tokio="$BASELINE" \
  --client "$CLIENT" --output "$OUTPUT" --mode control \
  --cases 1:1:64 1:32:64 --repeats 5
```

For the wider screen, use `--cases 1:1:4096 1:32:4096 4:1:64 4:1:4096 4:32:64 4:32:4096 --repeats 3` with a different fresh output directory. The runner records script, server, and client hashes under the shared experiment lock. Rebuilding the client creates a different experiment and requires recording its new identity.
