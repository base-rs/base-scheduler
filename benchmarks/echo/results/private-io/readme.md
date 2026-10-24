# Private I/O boundary comparison

This experiment checks whether implementing the six bound reader/writer capabilities directly in each adapter changes TCP echo throughput. The baseline is `39a61fa44adb17f7de10447a3b036b6e4847ea19`, before the final private-backend cleanup. The borrowed cases reuse a connection-local buffer; the owned cases transfer its owner for each operation. These exercise different Native I/O mechanisms, not just different argument types.

## Method

Aiko ran Linux `7.2.8-arch1-2` with Rust `1.99.0`. All servers were optimized release builds using the workspace lockfile. Server workers used CPUs 0–3; four fixed client workers used CPUs 4–7. Each sample used a one-second warmup and three-second measurement, with 64-byte round trips. Cases and variants were shuffled with seed `20261024`. CPU topology, executable hashes, source hashes, and every sample are retained in the JSON files.

The client executable stayed fixed, with SHA-256 `c72f7aea7151fb83020b096cfdaccc46a70b0eeff6bbb0a0e4ea4157d1aeb071`. These are closed-loop loopback measurements. They do not represent HTTP, physical-network throughput, or a guarantee about other message sizes.

## Expanded comparison

The [108-sample matrix](matrix.json) contains four repetitions of nine variants across three cases. Its source hash identifies a working-tree snapshot after the owned-write correction, before the final monitor construction and ownership simplifications. Values below are median requests per second; changes compare each adapter with its matching baseline.

| Workers / connections | Native borrowed before → after | Change | Native owned before → after | Change | Tokio adapter borrowed change | Tokio adapter owned change | Pure Tokio |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 / 1 | 276,179 → 271,755 | −1.60% | 232,680 → 239,502 | +2.93% | −0.81% | +0.58% | 277,010 |
| 1 / 32 | 632,883 → 625,684 | −1.14% | 625,840 → 612,724 | −2.10% | +0.37% | −2.02% | 707,402 |
| 4 / 32 | 1,747,378 → 1,729,698 | −1.01% | 1,714,324 → 1,709,176 | −0.30% | −1.07% | −0.11% | 1,748,005 |

The borrowed Native path is about 1–2% lower in this run. Some before/after sample ranges overlap, but the owned one-worker/32-connection ranges do not; the Tokio adapter owned control also declined about 2% there. These results do not establish that every difference is noise.

## Owned-path confirmation

The [54-sample confirmation](confirmation.json) repeats Native owned and pure Tokio six times per case using cleanup commit `63d0d5d78b7a5cf4b6e2f15e9f98085fa3a73355`. Later changes remove private monitor application-owner counting during construction/drop, remove unreachable checks, and extend tests; this is not a measurement of those final edits.

| Workers / connections | Native owned before | Native owned after | Change | Pure Tokio |
| --- | --- | --- | --- | --- |
| 1 / 1 | 238,150 | 230,711 | −3.12% | 275,167 |
| 1 / 32 | 621,060 | 617,320 | −0.60% | 724,723 |
| 4 / 32 | 1,728,704 | 1,707,360 | −1.23% | 1,728,911 |

The one-connection owned result changes direction between the two runs, with substantial baseline variation. The cleanup has small measured slowdowns in the confirmation, rather than a demonstrated throughput improvement. Native owned remains appreciably behind pure Tokio with one worker; removing the public backend layer does not resolve that existing difference.

The first confirmation attempt stopped when the pure Tokio control produced no complete requests in its measurement window. Its [partial samples](confirmation-invalid.json) and [failure log](confirmation-invalid.log) are retained separately and excluded from the completed confirmation. The subsequent full run completed using the same client executable.

## Regression found and corrected

The [preliminary matrix](preliminary.json) caught Native owned slowdowns of 8.2%, 4.8%, and 0.7%. The first cleanup unnecessarily started owned-operation tracking before a synchronous-ready send. The corrected path tries the send first and starts tracking only when backend work can outlive the call. This preserves lane ownership and safe close without adding that bookkeeping to immediately completed sends. Borrowed-only operations do not allocate the lazy owned-operation state.

Removing intermediate state also reduced the measured Native reader from 192 to 168 bytes, writer from 128 to 112 bytes, and borrowed-read future from 680 to 472 bytes. Owned-read future size stayed at 1,000 bytes; owned-write future changed from 992 to 984 bytes. These sizes describe the measured optimized snapshots, not stable ABI guarantees.

[compare.py](compare.py) and [confirm.py](confirm.py) preserve the research harnesses and machine-local executable paths. Recreate the recorded baseline and measured binaries before rerunning; the paths deliberately do not build or select a different revision automatically. The standard [echo runner](../../../run.py) describes the workload and portable invocation.
