# Worker hot-path measurements

All rates are median completed echo exchanges per second.

| Experiment | Workers/connections | Owned before → candidate | Change | Borrowed before → candidate | Change | Tokio before → candidate control | Change |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| single-local | 1/1 | 236,460 → 235,403 | -0.45% | 234,531 → 239,947 | +2.31% | 272,150 → 270,567 | -0.58% |
| single-local | 1/32 | 637,530 → 636,126 | -0.22% | 571,654 → 572,691 | +0.18% | 703,000 → 704,510 | +0.21% |
| single-local | 4/128 | 2,095,171 → 2,087,808 | -0.35% | 2,119,760 → 2,116,993 | -0.13% | 2,109,505 → 2,122,914 | +0.64% |
| injector-empty | 1/1 | 239,197 → 245,841 | +2.78% | 238,737 → 244,481 | +2.41% | 274,191 → 271,897 | -0.84% |
| injector-empty | 1/32 | 635,654 → 637,337 | +0.26% | 572,076 → 572,864 | +0.14% | 702,600 → 698,707 | -0.55% |
| injector-empty | 4/128 | 2,088,534 → 2,100,424 | +0.57% | 2,118,763 → 2,123,483 | +0.22% | 2,137,806 → 2,135,390 | -0.11% |

## Five-repetition single-connection confirmation

| Backend | Before | Candidate | Change |
| --- | ---: | ---: | ---: |
| iouring | 238,139 | 245,560 | +3.12% |
| iouring-poll | 231,411 | 236,820 | +2.34% |
| Unchanged Tokio control | 271,754 | — | — |
