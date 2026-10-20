# Native worker local wake permit variants

Both experimental variants start from c4f7eef (same-thread suppression on selected implementation) and add a per-worker AtomicBool local permit. A same-thread wake sets the permit after clearing sleeping. The flag is consumed only by that owning thread before an indefinite turn. skip omits the turn entirely; poll replaces it with a zero-timeout turn. Remote wakes and peer fanout retain previous behavior. No other runtime optimization was applied.

Three randomized repetitions per plain executable, one worker, one connection, 64 bytes, 1 s warmup plus 3 s measurement. Client binary fixed. Builds held shared /tmp/core-scheduler-perf.lock; the entire timing matrix held the exclusive lock. Release optimizations enabled with profile debug info and RUSTFLAGS unset.

| Variant | Median measured req/s | Median lifecycle voluntary CSW | req/s samples |
|---|---:|---:|---|
| selected-frozen | 238,077 | 67,177 | 237,485, 238,077, 239,464 |
| selected-fixed | 174,671 | 692,670 | 177,426, 171,602, 174,671 |
| selected-permit-skip | 185,913 | 757,819 | 185,913, 181,517, 191,857 |
| selected-permit-poll | 188,658 | 745,587 | 184,585, 200,280, 188,658 |

Changes relative to frozen selected implementation:

- selected-fixed: -26.63%.
- selected-permit-skip: -21.91%.
- selected-permit-poll: -20.76%.

The worker-level permit is not semantically identical to the selector pipe byte: driver.turn(None) can process new commands or CQEs and return without ever reaching its actual wait decision. A worker-level permit consumed before that turn can therefore be spent too early. The pipe implementation only consumes its pending bytes once turn has no other progress and is about to wait. A selector-level local permit consumed at that same location is the more faithful follow-up; this experiment makes no such selector change.

Counts are lifecycle totals including warmup and shutdown; throughput covers the measured client window. No counter-instrumented binaries were used in these timing samples. No coverage was run and no commits were pushed. Existing native/io_uring tests passed in the preceding owner-thread suppression experiment; this follow-up is a performance screen of experimental permit scheduling.
