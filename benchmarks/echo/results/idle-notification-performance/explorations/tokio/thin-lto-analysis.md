# Matched thin-LTO diagnostic

Source: clean `a98f172f8b06f23b34a354a4b6f6c294aa5859ab` for both binaries. No source or workspace profile changes.

Default release build: `cargo build --release --locked -p core-scheduler-echo-benchmark`.

Thin-LTO release build: `CARGO_PROFILE_RELEASE_LTO=thin cargo build --release --locked -p core-scheduler-echo-benchmark`, with an isolated `CARGO_TARGET_DIR`. All other profile settings remain Cargo defaults. Surrounding shell had no RUSTFLAGS, encoded flags, compiler wrapper, or Cargo release-profile overrides.

Compiler: rustc 1.99.0 (b940084d7 2026-09-28) (Arch Linux rust 1:1.99.0-1).

| Profile | Binary bytes | SHA-256 |
|---|---:|---|
| baseline | 2,194,000 | `c461c4a6c690f83823148970badfdf3cc4a30a06157c4edfeea66f8777c61f59` |
| thin-lto | 2,098,112 | `21af5d1e4ff8774150f550eaea5744031ca49b4906b1fd0198c383854b175e32` |

The LTO binary is about 4.4% smaller. `nm -C` no longer exposes Cancellation::check/is_cancelled, TokioScheduler::check_socket, or the scalar Tokio socket_io recv/send wrappers. This demonstrates optimization changes and does not itself establish a throughput improvement.

Timing uses all five backends, one worker, 32 connections, 64-byte messages, 3 randomized repeats per profile/backend (30 samples), 1 second warmup and 3 seconds measurement per sample. Both profiles use the identical frozen external client and CPU pinning. The runner holds the global performance lock exclusively.

All 30 samples completed successfully. Values below are median requests/second, followed by the observed minimum–maximum across three repeats. Relative columns compare each backend with pure Tokio built using the same profile.

| Backend | Default release | Thin LTO | Throughput change | Default / Tokio | Thin LTO / Tokio |
|---|---:|---:|---:|---:|---:|
| tokio | 697,481 (695,563–707,125) | 704,078 (702,442–705,278) | +0.95% | 100.00% | 100.00% |
| tokio-adapter | 646,027 (644,807–651,031) | 653,849 (649,455–656,419) | +1.21% | 92.62% | 92.87% |
| tokio-adapter-poll | 647,820 (640,003–649,124) | 649,960 (649,153–653,337) | +0.33% | 92.88% | 92.31% |
| iouring | 631,585 (627,126–637,867) | 633,643 (633,088–634,852) | +0.33% | 90.55% | 90.00% |
| iouring-poll | 567,878 (561,531–571,059) | 568,365 (567,368–572,679) | +0.09% | 81.42% | 80.72% |

Thin LTO changed throughput by +0.09% to +1.21% across these backend medians. With three repeats at one workload, these small gains do not establish a general performance improvement that justifies changing the project’s release profile. The adapter gap also remains: the owned Tokio adapter is 92.87% of same-profile Tokio with thin LTO, versus 92.62% with defaults; the borrowed adapter is 92.31% versus 92.88%. Keep the project profile unchanged.

This one-workload diagnostic cannot establish performance across payload sizes or worker counts. It tests the unchanged baseline commit; the separately retained native eventfd optimization is not included.
