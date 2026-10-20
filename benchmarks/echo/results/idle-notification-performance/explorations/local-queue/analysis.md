# Capture local queue once per driver turn

Decision: reject. All four matched median changes fall between -0.41% and +0.12%; the route-wrapper abstraction does not justify measurable complexity without a meaningful performance gain.

Base: `a98f172f8b06f23b34a354a4b6f6c294aa5859ab`. Optimized release build, no extra LTO/CPU flags. Three randomized repeats per case, 1 second warmup and 3 second timed measurement. CPU figures cover complete server lifetime including warmup/shutdown.

| Backend | Workers/connections | Before req/s (range) | After req/s (range) | Change | Before user/system s | After user/system s |
|---|---:|---:|---:|---:|---:|---:|
| iouring | 1/32 | 629,003 (626,430–629,891) | 629,755 (628,071–637,685) | +0.12% | 0.529/2.109 | 0.542/2.086 |
| iouring-poll | 1/32 | 566,461 (565,493–572,444) | 564,153 (564,005–566,580) | -0.41% | 0.547/2.247 | 0.533/2.250 |
| iouring | 4/128 | 2,060,571 (2,058,646–2,063,436) | 2,056,590 (2,056,577–2,068,334) | -0.19% | 2.798/8.641 | 2.757/8.705 |
| iouring-poll | 4/128 | 2,063,685 (2,059,239–2,066,304) | 2,059,261 (2,057,334–2,066,326) | -0.21% | 2.760/8.806 | 2.768/8.760 |

Archived exact patch: `source.patch`; baseline/candidate executable and build hashes/settings: `provenance.json`; complete raw measurements and helper/client provenance: `aiko.json`. Source prototype reverted. No production source changes or commit; no additional tests required for rejected prototype.
