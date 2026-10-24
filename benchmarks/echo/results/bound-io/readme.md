# Scheduler-bound I/O interface

These 60 randomized samples compare the original persistent monitor loop against the new scheduler-bound stream reader/writer loop. The latter retains the same nonblocking syscalls, persistent readiness registrations, and worker policy. No environment-controlled runtime prototype is enabled.

Aiko, Linux, optimized release builds with debug information, no LTO or CPU-specific overrides. Server CPUs 0–3, frozen validating client CPUs 4–7, one-second warmup and three-second measurement. Four repetitions of each backend/case, 64-byte requests; the complete source revisions, binary SHA-256 hashes, client identity, throughput, and latency samples are retained in [echo.json](echo.json). The unchanged [confirmation sampler](../parity/confirm.py) drives both source variants. The baseline is the cooperative-policy restoration `bef1940`; the bound interface is `229bde1`. Later changes to type names, factory abstraction, documentation, and file I/O were not part of these binaries.

| Workers / connections | Native monitor | Native bound handles | Change | Tokio monitor | Tokio bound handles | Change | Raw Tokio |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 / 1 | 272,444 | 273,923 | +0.54% | 270,082 | 273,936 | +1.43% | 274,272 |
| 1 / 32 | 631,339 | 626,428 | −0.78% | 648,077 | 644,498 | −0.55% | 715,195 |
| 4 / 32 | 1,738,324 | 1,746,039 | +0.44% | 1,677,585 | 1,671,786 | −0.35% | 1,743,538 |

Values are median requests/second. Each before/after pair has overlapping sample ranges; this small experiment shows no clear interface regression or established gain. The one-worker/32-connection Native gap to raw Tokio remains about 12.4%. The abstraction change does not establish general Tokio parity or measure packet/file performance.
