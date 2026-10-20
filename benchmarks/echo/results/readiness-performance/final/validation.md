# Verification

The performance executables use selected source `958ac6cf9862fdce2fc8de18c8864c4f971b6abc`. The subsequent `57af44e` changes only Linux conformance fixture ring sizes; production defaults and the measured runtime are identical.

| Host | Measured source regions covered | Invariant-only exclusions |
| --- | ---: | ---: |
| Aiko, Linux x86-64 | 18,497 / 18,497 | 1 |
| macOS ARM64 | 15,891 / 15,891 | 0 |

Both hosts passed workspace formatting, Clippy with warnings denied, documentation with warnings denied, and the canonical all-features/all-targets coverage task. Linux used `57af44e` source; macOS used `958ac6c` source, followed by a formatting check of the Linux-only fixture change. [Linux coverage](logs/coverage.log) and [macOS coverage](logs/macos-coverage.log) retain the full output. Other verification logs are alongside them.

The Linux coverage tool wrapper excludes only compiler sources installed under Arch's `/usr/src/debug/rust/`; it adds no project-source exclusions. The single invariant-only `unreachable!()` exclusion comes from the shared coverage task's normal policy.

Two earlier Linux attempts failed while constructing io_uring fixtures with `ENOMEM`: [default concurrency](logs/coverage-memlock-limit.log) and [eight test threads](logs/coverage-eight-threads-failed.log). Aiko's locked-memory limit is 8 MiB. Reducing the three shared conformance runners' ring sizes from 256 to 16 entries allowed the complete suite to pass at default concurrency. This is consistent with kernel ring-memory pressure during many short-lived fixtures, without proving the precise kernel accounting or teardown cause. Admission capacity, production defaults, and separate queue-saturation tests are unchanged. [The verification script](run-coverage.sh) records the successful environment.

Run `python3 benchmarks/echo/results/readiness-performance/final/summarize.py` from the repository root to verify the 150 final samples, individual JSON agreement, empty stderr, and source/runner/binary provenance, then regenerate the summary and tables. Exploratory runs are intentionally kept separate.
