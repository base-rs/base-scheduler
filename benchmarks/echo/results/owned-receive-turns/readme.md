# Coalescing Native owned receive turns

This change prepares scalar owned stream receives before publishing them to the ring owner, and avoids a nonblocking driver probe when the worker can prepare, submit, and wait in its normal idle turn. Busy workers still publish completions between ready batches. Owned reads continue to submit io_uring `Recv` and retain their buffers through terminal completion.

The buffer enters its stable request allocation before its pointer is derived. SQE preparation then uses the cached pointer and length without invoking user accessors on the ring owner. This preserves the driver's rule that preparation which can wake arbitrary tasks must return to the executor before waiting.

## Measurements

The [raw matrix](aiko.json), [serial confirmation](serial-confirmation.json), [build manifest](build-manifest.json), and [generated summary](summary.json) retain every sample and executable identity. Run `python3 benchmarks/echo/results/owned-receive-turns/summarize.py` to verify the complete matrices, frozen client, server identities, and per-cell throughput and P99 summaries. Raw samples are compacted onto individual JSON lines without changing their parsed contents.

The seven-repetition serial confirmation measures a 3.03% throughput improvement at one worker, one connection, and 64 bytes. Native remains 6.85% behind Tokio in that screen. Owned P99 is effectively unchanged; Tokio still has substantially lower P99. The frozen Native baseline moves from 240,882 echoes/s in the wide screen to 248,078 in the confirmation, so the screens must retain their own matched controls.

| Serial confirmation | Echoes/s | Per-run rate range | P99 |
| --- | ---: | ---: | ---: |
| Owned baseline | 248,078 | 242,720–250,820 | 5.703 µs |
| Owned candidate | 255,601 | 248,466–261,488 | 5.683 µs |
| Tokio | 274,408 | 272,387–276,674 | 4.303 µs |

In the wide screen, 64-byte single-connection owned reads improve by 4.25% with one worker and 6.90% with four workers. Other owned throughput changes range from −1.07% to +0.35%; borrowed throughput changes range from −0.70% to +1.29%. The change does not establish a general throughput gain across payloads. Busy owned P99 rises by 0.4–2.6 microseconds in this screen, up to 4.1%; the three-run ranges are retained in the summary. These small mixed changes should remain visible alongside the serial improvement.

Rates are validated echoes per second, and P99 values are microseconds. The case is workers/connections/payload bytes.

| Case | Owned baseline | Owned candidate | Owned change | Owned P99, baseline → candidate | Borrowed change | Borrowed P99, baseline → candidate |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1/1/64 | 240,882 | 251,119 | +4.25% | 5.711 → 5.703 | +0.97% | 4.351 → 4.291 |
| 1/1/4096 | 232,077 | 232,752 | +0.29% | 5.663 → 5.403 | −0.70% | 5.323 → 5.303 |
| 1/32/64 | 657,661 | 650,628 | −1.07% | 52.863 → 53.279 | +0.58% | 55.359 → 55.135 |
| 1/32/4096 | 590,879 | 586,225 | −0.79% | 58.623 → 60.031 | +0.87% | 61.311 → 60.799 |
| 4/1/64 | 239,696 | 256,240 | +6.90% | 6.091 → 5.751 | +0.94% | 4.711 → 4.711 |
| 4/1/4096 | 233,752 | 234,570 | +0.35% | 6.051 → 6.063 | +0.46% | 5.451 → 5.563 |
| 4/32/64 | 1,737,405 | 1,727,195 | −0.59% | 53.759 → 55.327 | −0.29% | 52.511 → 54.399 |
| 4/32/4096 | 1,605,283 | 1,605,563 | +0.02% | 62.623 → 65.215 | +1.29% | 62.271 → 63.135 |

## Method

The source comparison is `4c1db9d6569a284aaf696225d2f48235612927a3` against `0e05e3f2c8edf091b732c9a5b6a2d27608db11f3`. Both frozen executables were built in the same disposable checkout, replacing only the two production Rust files between builds. Tests and documentation do not enter the benchmark binary. Both builds use locked dependencies, opt-level 3, 16 codegen units, debug information, no LTO, no incremental compilation, and no additional Rust flags. Later commits only add this evidence.

The host is Aiko's Ryzen 9 9950X3D running Linux 7.2.8 and Rust 1.99.0 / LLVM 23.1.1. Server CPUs are 0 through workers minus one; the fixed four-worker client uses CPUs 4–7, separate physical cores in the same L3 group. The governors remain `powersave`. Builds, tests, and benchmarks serialize on `/tmp/core-scheduler-perf.lock`.

The wide screen has three shuffled repetitions of each of eight cases and five variants: 120 uninstrumented samples. A separate serial confirmation repeats the one-worker, one-connection, 64-byte case seven times for owned baseline, owned candidate, and Tokio: another 21 samples. Each sample has one second of warmup and three measured seconds. This is closed-loop IPv4 loopback echo with one outstanding request per connection, reused initialized 64 KiB buffers, `TCP_NODELAY`, complete partial writes, and validated payloads. Each rate and P99 in the summary is the median of the per-run values in its own screen; P99s are not pooled across runs.

The owned variants use backend `iouring`, the borrowed variants use `iouring-monitor`, and the Tokio control uses `tokio` from the frozen baseline executable. The borrowed variants remain separate controls rather than substitutions for completion-based receives. Ordinary Tokio and Native also retain their existing read-policy differences. Compare candidate against its matching Native baseline to assess this change; the Tokio column does not isolate every semantic or executor difference. Three repetitions on one host do not establish sub-percent improvements or production tail latency.

## Reproduction

Use disposable checkouts at the two source commits, the same Cargo target directory and release settings from the build manifest, and `cargo build --release --locked -p core-scheduler-echo-benchmark`. Copy each resulting executable to a separate frozen path before building the next variant. The recorded client remains at `/home/samuel/.cache/core-scheduler-single-worker-completions/bin/baseline` on Aiko; using a rebuilt client is a different experiment and requires recording its identity.

Run the existing [measurement runner](../worker-completions/measure.py) from the candidate checkout, supplying the two frozen server paths as `BASELINE` and `CANDIDATE`, the validating client as `CLIENT`, and a new output directory as `OUTPUT`:

```sh
python3 benchmarks/echo/results/worker-completions/measure.py \
  --variant owned-baseline=iouring="$BASELINE" \
  --variant owned-candidate=iouring="$CANDIDATE" \
  --variant borrowed-baseline=iouring-monitor="$BASELINE" \
  --variant borrowed-candidate=iouring-monitor="$CANDIDATE" \
  --variant tokio=tokio="$BASELINE" \
  --client "$CLIENT" --output "$OUTPUT" --mode control \
  --cases 1:1:64 1:1:4096 1:32:64 1:32:4096 4:1:64 4:1:4096 4:32:64 4:32:4096 \
  --repeats 3
```

The runner acquires the shared experiment lock, records binary and script hashes, and refuses duplicate backend/executable identities. Preserve raw profiles, executables, and build targets outside the repository.

For the serial confirmation, omit the two borrowed variants and use `--cases 1:1:64 --repeats 7` with another fresh output directory. The summary validates the two screens separately; their medians are not combined.
