# Scheduler-bound I/O interface

These 60 randomized samples compare the original persistent monitor loop against the new scheduler-bound stream reader/writer loop. The latter retains the same nonblocking syscalls, persistent readiness registrations, and worker policy. No environment-controlled runtime prototype is enabled.

Aiko, Linux, optimized release builds with debug information, no LTO or CPU-specific overrides. Server CPUs 0–3, frozen validating client CPUs 4–7, one-second warmup and three-second measurement. Four repetitions of each backend/case, 64-byte requests; the complete source revisions, binary SHA-256 hashes, client identity, throughput, and latency samples are retained in [echo.json](echo.json). The unchanged [confirmation sampler](../parity/confirm.py) drives both source variants. The baseline is the cooperative-policy restoration `bef1940`; the bound interface is `229bde1`. Later changes to type names, factory abstraction, documentation, and file I/O were not part of these binaries.

| Workers / connections | Native monitor | Native bound handles | Change | Tokio monitor | Tokio bound handles | Change | Raw Tokio |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 / 1 | 272,444 | 273,923 | +0.54% | 270,082 | 273,936 | +1.43% | 274,272 |
| 1 / 32 | 631,339 | 626,428 | −0.78% | 648,077 | 644,498 | −0.55% | 715,195 |
| 4 / 32 | 1,738,324 | 1,746,039 | +0.44% | 1,677,585 | 1,671,786 | −0.35% | 1,743,538 |

Values are median requests/second. Each before/after pair has overlapping sample ranges; this small experiment shows no clear interface regression or established gain. The one-worker/32-connection Native gap to raw Tokio remains about 12.4%. The abstraction change does not establish general Tokio parity or measure packet/file performance.

## Opaque factory confirmation

A second 60-sample randomized comparison measures the completed factory implementation at `49b2b7b695629b153b5847177ed41c39df2630a3`, including backend-selected opaque factories and Native forwarding. The later source commit `9b49757` adds only cancellation tests. The baseline remains the original monitor interface at `bef1940`. Both binaries use `CARGO_PROFILE_RELEASE_DEBUG=1 cargo build --release`, with the same client, CPU affinity, warmup, measurement duration, and four repetitions per case described above. [Factory samples](final.json) retain every repetition and both executable identities.

| Workers / connections | Native monitor | Native bound handles | Change | Tokio monitor | Tokio bound handles | Change | Raw Tokio |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 / 1 | 271,513 | 271,914 | +0.15% | 271,949 | 274,141 | +0.81% | 273,637 |
| 1 / 32 | 632,666 | 633,331 | +0.11% | 646,854 | 650,543 | +0.57% | 705,060 |
| 4 / 32 | 1,746,211 | 1,748,112 | +0.11% | 1,678,070 | 1,685,234 | +0.43% | 1,731,547 |

Every before/after sample range overlaps again. These small changes provide no clear evidence of a throughput regression or gain from the interface redesign. Native's one-worker/32-connection median remains 10.2% below raw Tokio in this run; its one-connection and four-worker results are close to the control. The smaller gap than in the first run is not evidence that the redesign closed it: Native's matched before/after change is only 0.11%, while the raw Tokio control differs between runs. These remain short loopback echo measurements, not packet, file, HTTP, or general workload results.

## Portable empty-read behavior

Linux and Windows conformance exposed an idle zero-capacity read waiting for data. The final executable source at `108252e321931d48afb8be6254b64806b970e534` returns zero for empty stream destinations after checking local handle state and cancellation, without establishing EOF. Nonempty stream reads retain the same persistent readiness path, and packet behavior is unchanged. The later `959e37b` commit changes documentation only. Another 60 randomized samples use the same optimized build configuration, original monitor baseline, frozen client, and sampling settings. [Portable-reader samples](portable.json) retain all measurements.

| Workers / connections | Native monitor | Native bound handles | Change | Tokio monitor | Tokio bound handles | Change | Raw Tokio |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 / 1 | 273,603 | 273,832 | +0.08% | 271,422 | 271,576 | +0.06% | 273,578 |
| 1 / 32 | 632,352 | 632,954 | +0.10% | 647,841 | 643,728 | −0.63% | 711,333 |
| 4 / 32 | 1,764,197 | 1,748,242 | −0.90% | 1,681,014 | 1,688,507 | +0.45% | 1,735,415 |

Native's one-worker/32-connection median remains 11.0% below raw Tokio. Five of the six before/after ranges overlap; the four-worker Native samples are all below the baseline samples in this run. The 0.9% median reduction therefore receives a separate confirmation rather than being dismissed as noise.

The follow-up uses eight repetitions of the four-worker/32-connection case for each frozen executable, including both bound-interface revisions and raw Tokio, for 32 additional randomized samples. Build identities and sampling settings are unchanged; [confirmation.json](confirmation.json) retains the complete run.

| Executable / path | Median requests/second | Change from original Native |
| --- | ---: | ---: |
| Original Native monitor (`bef1940`) | 1,754,275 | — |
| Opaque factory handles (`49b2b7b`) | 1,763,071 | +0.50% |
| Portable stream reader (`108252e`) | 1,764,366 | +0.58% |
| Raw Tokio control | 1,750,354 | — |

All Native sample ranges overlap in the confirmation. The portable reader is 0.07% above the earlier factory implementation by median; the earlier 0.9% decrease does not repeat. These experiments do not establish a repeatable four-worker regression or improvement. Across the measured echo cases, the new application interface preserves throughput within small run-to-run changes, while the one-worker concurrent gap to raw Tokio remains unresolved. Packet and file throughput are not measured by these echo runs.
