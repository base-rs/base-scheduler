# Native idle policy regression screens

Recommendation: retain the original native interrupt and wait policy for the shipping change. None of these small alternatives removes the owned serial regression without giving back other gains. Keep confirmed capacity/drain notification reductions separately.

Each stage uses three randomized repetitions, 1-second warmup plus 3-second measurement, server CPU 0, fixed client CPUs 4–7. Frozen before/current controls are rerun inside each stage; compare within a stage. Builds use release opt-level 3 and RUSTFLAGS=-Cdebuginfo=1. Shared flock for builds; exclusive flock for timed samples. These are screening results, not replacements for the original 300-sample matrix.

| Screen | Backend | Connections | Variant | Median req/s | Range req/s | Lifecycle server CPU s | Median p99 µs |
| --- | --- | ---: | --- | ---: | ---: | ---: | ---: |
| ppoll | tokio | 1 | before | 272,852 | 270,298–274,079 | 1.620 | 4.363 |
| ppoll | iouring | 1 | after | 200,553 | 196,050–220,939 | 2.382 | 7.583 |
| ppoll | iouring | 1 | before | 237,237 | 234,521–240,851 | 3.360 | 5.943 |
| ppoll | iouring | 1 | ppoll | 187,855 | 181,992–188,461 | 2.337 | 10.791 |
| ppoll | iouring-poll | 1 | after | 210,763 | 208,578–217,266 | 1.712 | 9.607 |
| ppoll | iouring-poll | 1 | before | 178,958 | 175,151–180,235 | 2.108 | 10.647 |
| ppoll | iouring-poll | 1 | ppoll | 194,194 | 193,608–197,551 | 1.807 | 9.903 |
| ppoll | tokio | 32 | before | 697,452 | 695,723–702,387 | 2.544 | 53.631 |
| ppoll | iouring | 32 | after | 528,728 | 526,332–533,037 | 2.870 | 68.351 |
| ppoll | iouring | 32 | before | 517,829 | 516,674–520,798 | 2.883 | 69.759 |
| ppoll | iouring | 32 | ppoll | 535,931 | 535,412–542,182 | 2.846 | 67.135 |
| ppoll | iouring-poll | 32 | after | 563,361 | 562,403–566,103 | 2.782 | 62.687 |
| ppoll | iouring-poll | 32 | before | 559,065 | 557,599–561,870 | 2.803 | 63.295 |
| ppoll | iouring-poll | 32 | ppoll | 569,344 | 567,638–570,610 | 2.778 | 62.143 |
| read | tokio | 1 | before | 272,903 | 271,115–272,948 | 1.618 | 4.323 |
| read | iouring | 1 | after | 215,171 | 198,168–220,013 | 2.562 | 6.791 |
| read | iouring | 1 | before | 235,713 | 235,608–238,097 | 3.361 | 6.031 |
| read | iouring | 1 | read | 191,093 | 189,737–196,390 | 2.119 | 7.323 |
| read | iouring-poll | 1 | after | 208,043 | 207,479–209,628 | 1.710 | 7.431 |
| read | iouring-poll | 1 | before | 182,593 | 182,528–183,233 | 2.114 | 10.511 |
| read | iouring-poll | 1 | read | 231,541 | 222,867–235,166 | 1.827 | 7.191 |
| read | tokio | 32 | before | 696,160 | 695,032–697,180 | 2.546 | 53.695 |
| read | iouring | 32 | after | 526,934 | 525,836–529,775 | 2.852 | 68.351 |
| read | iouring | 32 | before | 518,284 | 518,020–520,016 | 2.874 | 69.631 |
| read | iouring | 32 | read | 536,324 | 533,444–541,419 | 2.842 | 67.071 |
| read | iouring-poll | 32 | after | 564,508 | 564,380–568,784 | 2.786 | 62.463 |
| read | iouring-poll | 32 | before | 560,428 | 560,389–564,229 | 2.802 | 63.103 |
| read | iouring-poll | 32 | read | 568,895 | 567,535–570,902 | 2.785 | 62.143 |
| stream | tokio | 1 | before | 273,258 | 271,115–273,647 | 1.619 | 4.343 |
| stream | iouring | 1 | after | 204,364 | 201,010–210,939 | 2.449 | 10.471 |
| stream | iouring | 1 | before | 234,883 | 233,402–236,540 | 3.364 | 6.103 |
| stream | iouring | 1 | stream | 222,601 | 215,928–226,093 | 2.884 | 7.711 |
| stream | iouring-poll | 1 | after | 212,594 | 208,240–220,748 | 1.724 | 9.671 |
| stream | iouring-poll | 1 | before | 181,152 | 178,060–182,928 | 2.115 | 10.567 |
| stream | iouring-poll | 1 | stream | 204,422 | 202,343–204,744 | 1.706 | 7.411 |
| spin | tokio | 1 | before | 271,947 | 268,083–273,702 | 1.624 | 4.363 |
| spin | iouring | 1 | after | 213,855 | 201,515–220,180 | 2.597 | 6.371 |
| spin | iouring | 1 | before | 236,970 | 236,208–237,357 | 3.389 | 5.883 |
| spin | iouring | 1 | spin | 196,205 | 189,637–199,888 | 2.501 | 6.971 |
| spin | iouring-poll | 1 | after | 209,493 | 204,196–218,198 | 1.697 | 6.183 |
| spin | iouring-poll | 1 | before | 181,369 | 178,650–182,768 | 2.120 | 10.391 |
| spin | iouring-poll | 1 | spin | 196,718 | 196,485–203,757 | 1.782 | 6.231 |

Interpretation: the event counter/direct-ring changes reduce CPU usage substantially but park more readily. Restoring the old stream notifier recovers some owned throughput while increasing CPU; it still leaves a reproducible roughly 5% owned gap. A direct eventfd Read improves borrowed serial throughput but worsens owned throughput, and adds userspace storage lifetime obligations. Sixteen bounded spin rechecks before parking also fails to recover owned throughput. This evidence supports rejecting the overhaul from the default shipping path; it does not establish a single causal kernel mechanism.

The direct Read prototype uses stable Box\<UnsafeCell<u64>> storage, explicitly cancels and awaits the original internal read CQE on orderly shutdown, and intentionally retains pending storage on exceptional drop. It was built and screened only, not promoted through the conformance/coverage gate. No prototype here should be shipped merely because it compiles.

See provenance.json for exact commits, binary paths/hashes, and raw results.

The retained `ppoll.patch`, `read.patch`, `stream.patch`, and `spin.patch` reproduce each experimental source tree from `4a061fbc4355738f39f5eb6ea1db1520db6a753e`. These variants were built and screened, and were not selected or given production conformance/coverage approval.
