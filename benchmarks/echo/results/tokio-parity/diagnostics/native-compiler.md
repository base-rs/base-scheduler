# Native compiler-boundary diagnostic

Baseline: commit `1840258bbf9cf10c08fefa26fbff54a747f18d08` plus the screened exclusive-monitor implementation, blob `7283980a9fc48bdbdb786117d7505e211c689851`. No production changes are selected by this diagnostic.

Aiko, locked optimized release builds with `CARGO_PROFILE_RELEASE_DEBUG=1`, source files force-touched before each build. One server worker on CPU 0, four frozen client workers on CPUs 4–7, 32 connections, 64-byte round trips, 1 second warmup and 3 measured seconds. Three randomized repetitions per variant; `/tmp/core-scheduler-perf.lock` covers all builds and timings.

| Variant | Median requests/s | Range |
| --- | ---: | ---: |
| exclusive | 634,734 | 633,989–636,009 |
| force-inline | 632,816 | 631,559–633,944 |
| thinlto | 638,891 | 638,775–641,583 |
| raw | 719,955 | 708,747–721,509 |
| raw-thinlto | 711,853 | 702,380–723,975 |
| conservative | 665,571 | 664,138–671,482 |

`force-inline` adds only five `#[inline(always)]` annotations: bound `read_into` and `write`, private `monitor_io`, and scalar monitor recv/send wrappers. Its executable's entire `.text` section is byte-identical to the exclusive baseline: 1,160,181 bytes, SHA256 `8c874dea4c815f437ae1be53bf579d25680cde19a85869d26a031a2cb8d8db3d`. The separate read\_into poll remains 8,133 bytes with a 408-byte stack frame. Timing differences between these identical machine-code variants are noise.

`thinlto` retains the exact exclusive source and adds only `CARGO_PROFILE_RELEASE_LTO=thin` to the build environment, preserving default codegen units. The read\_into poll shrinks to 7,699 bytes and its stack frame to 392 bytes, but Native throughput changes by only +0.65%. Raw Tokio in the same ThinLTO executable changes by -1.13%. This short screen does not establish a meaningful compiler-driven improvement.

The exclusive Native baseline is 4.63% behind the conservative pure Tokio receive policy in this screen. The normal path's disassembly shows nested poll calls and small reference-field copies, with no obvious large memcpy. Whole-program optimization does not eliminate the remaining gap, so focus subsequent research on actual wake/driver scheduling costs rather than assuming frame copies dominate.

`native-compiler-screen.json` preserves all 18 samples and executable, source-patch and client hashes. The shared comparison runner is `../compare.py`. ThinLTO is diagnostic only; no workspace profile change is recommended from these results.

## Fresh exclusive profile

After the two compiler diagnostics, a separate optimized `cycles:u` profile and syscall-counting run used the exact frozen exclusive baseline. The self-time profile, route assembly, and counters are retained alongside this note; binary perf data remains on Aiko.

The fresh user-cycle profile attributes 10.69% to `IoUringSelector::route`; 99.07% of that symbol's annotated samples land on the branch immediately following the TLS-selected controller's `Arc::clone` (`lock incq`). Other leading symbols are task run (12.50%), bound read\_into (7.98%), poll\_monitor\_wait (7.19%), notify (6.39%), worker run\_loop (4.18%) and schedule (4.14%). These are sampled attribution, not isolated measurements of each component.

This supports investigating resource-local cached controller ownership, refreshed on worker migration, rather than further async inlining. The current route is cloned and dropped for each stalled scalar read; persistent registration already retains a controller separately. Any such change must preserve original-ring ownership during an accepted operation, external-thread fallback routing, retirement and task migration. No route-cache prototype was implemented in this focused diagnostic.
