# Scheduler-bound I/O interface

These 60 randomized samples compare the original persistent monitor loop against the new scheduler-bound stream reader/writer loop. The latter retains the same nonblocking syscalls, persistent readiness registrations, and worker policy. No environment-controlled runtime prototype is enabled.

Aiko, Linux, optimized release builds with debug information, no LTO or CPU-specific overrides. Server CPUs 0–3, frozen validating client CPUs 4–7, one-second warmup and three-second measurement. Four repetitions of each backend/case, 64-byte requests; the complete source revisions, binary SHA-256 hashes, client identity, throughput, and latency samples are retained in [echo.json](echo.json). The unchanged [confirmation sampler](../parity/confirm.py) drives both source variants. The baseline is the cooperative-policy restoration `bef1940`; the bound interface is `229bde1`. Later changes to type names, factory abstraction, documentation, and file I/O were not part of these binaries.

| Workers / connections | Native monitor | Native bound handles | Change | Tokio monitor | Tokio bound handles | Change | Raw Tokio |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 / 1 | 272,444 | 273,923 | +0.54% | 270,082 | 273,936 | +1.43% | 274,272 |
| 1 / 32 | 631,339 | 626,428 | −0.78% | 648,077 | 644,498 | −0.55% | 715,195 |
| 4 / 32 | 1,738,324 | 1,746,039 | +0.44% | 1,677,585 | 1,671,786 | −0.35% | 1,743,538 |

Values are median requests/second. Each before/after pair has overlapping sample ranges; this small experiment shows no clear interface regression or established gain. The one-worker/32-connection Native gap to raw Tokio remains about 12.4%. The abstraction change does not establish general Tokio parity or measure packet/file performance.

## Final source confirmation

A second 60-sample randomized comparison measures the final production implementation at `49b2b7b695629b153b5847177ed41c39df2630a3`, including backend-selected opaque factories and Native forwarding. The later source commit `9b49757` adds only cancellation tests. The baseline remains the original monitor interface at `bef1940`. Both binaries use `CARGO_PROFILE_RELEASE_DEBUG=1 cargo build --release`, with the same client, CPU affinity, warmup, measurement duration, and four repetitions per case described above. [Final samples](final.json) retain every repetition and both executable identities.

| Workers / connections | Native monitor | Native bound handles | Change | Tokio monitor | Tokio bound handles | Change | Raw Tokio |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 / 1 | 271,513 | 271,914 | +0.15% | 271,949 | 274,141 | +0.81% | 273,637 |
| 1 / 32 | 632,666 | 633,331 | +0.11% | 646,854 | 650,543 | +0.57% | 705,060 |
| 4 / 32 | 1,746,211 | 1,748,112 | +0.11% | 1,678,070 | 1,685,234 | +0.43% | 1,731,547 |

Every before/after sample range overlaps again. These small changes provide no clear evidence of a throughput regression or gain from the interface redesign. Native's one-worker/32-connection median remains 10.2% below raw Tokio in this run; its one-connection and four-worker results are close to the control. The smaller gap than in the first run is not evidence that the redesign closed it: Native's matched before/after change is only 0.11%, while the raw Tokio control differs between runs. These remain short loopback echo measurements, not packet, file, HTTP, or general workload results.
