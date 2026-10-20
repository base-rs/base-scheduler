# Private request identifier hash experiment

Base: `a98f172f8b06f23b34a354a4b6f6c294aa5859ab`.

Decision: reject. Gains are below 1% in every matched median and do not justify another private hash implementation; the simple multiplicative screening hash also has poor low-bit stride distribution.

Three randomized repetitions; same optimized release build configuration, fixed client, CPU affinity, 1 second warmup + 3 second measurement. CPU figures cover the complete server process lifecycle including warmup and shutdown, not just the timed measurement.

| Backend | Workers/connections | Before req/s (range) | After req/s (range) | Change | Before user/system s | After user/system s |
|---|---:|---:|---:|---:|---:|---:|
| iouring | 1/1 | 231,362 (227,678–236,123) | 232,363 (232,222–235,154) | +0.43% | 0.405/1.395 | 0.383/1.413 |
| iouring-poll | 1/1 | 226,239 (224,944–229,583) | 227,966 (218,513–233,414) | +0.76% | 0.465/1.337 | 0.436/1.307 |
| iouring | 1/32 | 628,779 (626,685–630,115) | 633,847 (633,685–636,531) | +0.81% | 0.523/2.110 | 0.533/2.093 |
| iouring-poll | 1/32 | 565,654 (565,454–566,126) | 569,724 (568,790–569,928) | +0.72% | 0.538/2.241 | 0.511/2.265 |
| iouring | 4/128 | 2,043,851 (2,027,016–2,050,426) | 2,054,980 (2,049,825–2,071,743) | +0.54% | 2.710/8.744 | 2.720/8.712 |
| iouring-poll | 4/128 | 2,047,611 (2,042,281–2,057,042) | 2,054,949 (2,054,922–2,065,318) | +0.36% | 2.786/8.750 | 2.680/8.864 |

Exact candidate patch, baseline/candidate executable hashes, build logs and settings: `provenance.json`. Raw measurements and environment/helper metadata: `aiko.json`. The candidate was reverted after measurement. No retained source changes, no commit, and no additional tests were needed for a rejected experiment.
