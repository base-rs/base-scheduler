# Focused native owned serial 4 KiB check

Five randomized repeats each for baseline native, selected native, and baseline pure Tokio; one worker, one connection, 1-second warmup and 3-second measurement. Exclusive timing flock; no builds or source edits.

| Variant | Median req/s | Range req/s | Mean req/s | Median server lifecycle CPU s | Median p99 µs |
|---|---:|---:|---:|---:|---:|
| before iouring | 188,916 | 167,470–191,310 | 184,712 | 3.008 | 8.011 |
| after iouring | 186,524 | 183,803–196,132 | 189,266 | 3.070 | 7.663 |
| before tokio | 234,407 | 233,155–242,741 | 236,140 | 1.596 | 5.383 |

Selected native median change: -1.27%; mean change: +2.47%.

This focused check does not establish the original 4.9% drop as a reproducible effect. Native results still vary substantially and their ranges overlap. The median moves less than 4.9%, while the direction/size depends on summary statistic; the evidence remains inconclusive for a small regression. Treat selected native serial 4 KiB as broadly similar with unresolved variability, rather than claiming a demonstrated speedup or proving exact parity.

Exact binary/source/client identities and per-sample metadata are in aiko.json. Runner: [focus-selected-4k.py](focus-selected-4k.py). Original 180-sample and earlier 300-sample evidence remain unchanged.
