# Native borrowed serial cooperative-ring diagnostic

Five randomized repetitions per frozen binary, exclusive shared performance lock, one-second warmup and three-second measurement, default optimized release builds, fixed client, 1 worker/1 connection/64 bytes. The no-coop variant differs from selected source only by removing `setup_coop_taskrun` and `setup_taskrun_flag`; started tracking and replacement-ring construction remain.

| Binary | Median requests/s | Mean requests/s | Range requests/s |
| --- | ---: | ---: | ---: |
| before | 233,158 | 231,266 | 224,197–235,865 |
| selected | 227,074 | 227,887 | 223,232–233,437 |
| no-coop | 230,621 | 228,613 | 224,394–232,609 |

Selected is 2.61% below before by median but 1.46% by mean. Removing the cooperative flags recovers 1.56% by median and only 0.32% by mean, leaving no-coop 1.09% below before by median. All three sample ranges overlap substantially. These 15 samples support a small serial tradeoff being possible, but do not establish the previously observed 4.2% median gap as the causal cost of cooperative flags or justify an adaptive policy. Between-run variability is similar in scale to the observed differences.

`serial/aiko.json` and individual sampleJSON/stderr retain every sample; `summary.json` includes means, medians, ranges, RTT and lifecycleCPU/context-switch values. CPU/context-switch fields include warmup and teardown, while rates/RTT cover the measured window. `provenance.json` records binary/source/runner hashes, the exact isolated two-line flags delta, build settings/toolchain/topology and affinity. No production changes were selected or committed.
