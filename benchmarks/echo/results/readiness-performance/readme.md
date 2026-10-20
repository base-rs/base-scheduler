# Readiness performance evidence

`final/` contains the complete, optimized before/after matrix for the selected implementation. Run its `summarize.py` to validate sample completeness and regenerate `summary.json` and `tables.md`. The final source, build settings, binary hashes, and host details are recorded separately from the exploratory runs.

The [verification record](final/validation.md) covers the final source checks and labels the earlier Linux fixture failures separately from the passing run.

`explorations/` preserves diagnostics and rejected candidates. These samples must not be pooled with the final matrix:

- `tokio/counters/`: original receive-attempt counters.
- `tokio/short-read/`, `tokio/readiness-first/`, and `tokio/final-counters/`: **rejected short-read-clearing implementations**. Their faster results do not describe the selected runtime. A short TCP read can stop at an urgent-data boundary without draining the receive queue.
- `short-read-aborted/`: incomplete integration matrix stopped when the urgent-data regression was confirmed. No final conclusions use these samples.
- `urgent-data/`: failing regression and passing no-clear control for that rejected implementation.
- `raw-tokio-urgent/`: separate raw Tokio 1.53.3 reproduction using `AsyncRead::poll_read`, with a `readable()`/`try_read` control. Its log intentionally reports the reproduced timeout. No adapter I/O is invoked by the comparison.
- `readiness-policy/`: separate 15-sample comparison of raw Tokio receive loops and both selected adapter paths. The polling policy retains readiness until `WouldBlock`; this measures the effect of the raw loop and policy together, rather than one syscall in isolation.
- `tokio/readiness-only/`: pilot for the selected conservative readiness policy.
- `tokio/safe-counters/`: selected policy with TLS counters, verifying one failed receive per steady read instead of two. Instrumented throughput is not a capacity result.
- `tokio/inline-hints/`: rejected explicit-inlining experiment; no useful throughput improvement.
- `cooperative-ring/`: initial cooperative-task-work pilot. Its setup applied to movable drivers too; production narrows setup to fresh thread-bound workers and is measured again in the final matrix.
- `native-serial/`: separate 15-sample follow-up of the single-connection borrowed Native result, including an isolated cooperative-flags-disabled control. The overlapping ranges leave a possible small serial tradeoff unresolved.
- `read-first/` and `ring-poll-first/`: rejected synchronous-receive and kernel poll-first owned-read experiments.
- `native/`: rejected readiness-completion, route, and wake shortcuts, plus baseline profiles and analysis.
- `task-wrapper/`: diagnostic bypassing the task wrapper while preserving adapter I/O; no sufficient benefit to justify changing task ownership or panic handling.

Frozen executables and full binary perf recordings remain in Aiko's task cache; this directory stores text evidence, patches, samples, and hashes. Host paths in scripts refer to those original caches. Final lifecycle CPU values cover setup, warmup, measurement, and teardown; request rates cover only the measurement window.
