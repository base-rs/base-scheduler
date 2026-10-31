# Same-thread worker wake suppression experiment

The runtime patch retains sleeping.swap(false, SeqCst), publication fences, peer fanout, and remote interrupts. It skips unpark and driver_waker only when the registered target thread is the current thread. No other runtime changes were made.

Three randomized serial TCP echo samples per executable: one worker, one connection, 64 bytes, 1 s warmup plus 3 s measurement. All server context switches and source counters cover lifecycle including warmup and shutdown. Counter builds and plain timing builds are separate. Builds held shared /tmp/core-scheduler-perf.lock; the matrix held it exclusively.

| Variant | Median measured req/s | Median lifecycle voluntary CSW | req/s samples |
| --- | ---: | ---: | --- |
| prototype-frozen | 209,795 | 820,786 | 209,795, 199,023, 210,923 |
| prototype-fixed | 188,243 | 756,434 | 188,243, 188,848, 187,096 |
| selected-frozen | 231,874 | 195,265 | 231,874, 229,642, 237,379 |
| selected-fixed | 176,682 | 699,913 | 177,006, 172,859, 176,682 |
| prototype-frozen-counted | 212,419 | 853,772 | 212,419, 207,121, 220,263 |
| prototype-fixed-counted | 187,504 | 747,560 | 187,627, 185,628, 187,504 |
| selected-frozen-counted | 238,917 | 89,727 | 238,917, 214,122, 241,344 |
| selected-fixed-counted | 177,667 | 707,065 | 177,667, 170,073, 177,748 |

Plain fixed/frozen median throughput changes:

- prototype: -10.27%.
- selected: -23.80%.

| Counter variant/repeat | Payload CQEs / 2 | Same-worker notify success | Other-thread notify success | Interrupt SQEs/CQEs | Wait syscalls | Voluntary CSW |
| --- | ---: | ---: | ---: | --- | ---: | ---: |
| prototype-frozen-counted/1 | 854,108.5 | 831,147 | 2 | 830,890/830,890 | 853,953 | 853,772 |
| prototype-frozen-counted/2 | 856,754.5 | 833,703 | 3 | 833,493/833,493 | 856,629 | 856,457 |
| prototype-frozen-counted/3 | 812,097.5 | 790,262 | 2 | 790,051/790,051 | 811,973 | 811,823 |
| prototype-fixed-counted/1 | 742,512.5 | 0 | 2 | 2/2 | 742,476 | 742,467 |
| prototype-fixed-counted/2 | 747,600.5 | 0 | 2 | 2/2 | 747,568 | 747,560 |
| prototype-fixed-counted/3 | 753,855.5 | 0 | 2 | 2/2 | 753,816 | 753,802 |
| selected-frozen-counted/1 | 857,544.5 | 834,574 | 2 | 0/0 | 857,311 | 369,510 |
| selected-frozen-counted/2 | 960,131.5 | 934,402 | 2 | 0/0 | 959,894 | 89,727 |
| selected-frozen-counted/3 | 955,275.5 | 930,271 | 2 | 0/0 | 953,890 | 70,678 |
| selected-fixed-counted/1 | 692,336.5 | 0 | 2 | 0/0 | 692,322 | 692,237 |
| selected-fixed-counted/2 | 707,183.5 | 0 | 2 | 0/0 | 707,170 | 707,065 |
| selected-fixed-counted/3 | 711,823.5 | 0 | 2 | 0/0 | 711,801 | 711,676 |

The counter TLS marker classifies the installed worker thread; in this one-worker case that is the target worker. Payload Submit commands already enter the matching TLS queue and return before notify; Controller::Wake can notify independently. Untagged original CQEs in this workload are socket Recv/Send attempts, including any retries or partial transfers and final EOF; accept, registration and TCP_NODELAY use the readiness fallback. Payload CQEs / 2 estimates lifecycle RTTs. It is not an exact successful measured-request count.

The final EOF contributes one additional Recv with no matching Send. The odd .5 totals are consistent with this, but these aggregate counters do not independently establish absence of unsuccessful attempts. The selected implementation uses stream notifications and readiness poller, so it has no interrupt SQE/CQE. Its wait_function counter remains unused; wait_syscall is the relevant wait count.

Source counters prove whether same-worker notification calls were removed. They do not prove each blocking-capable wait actually slept; voluntary context switches are a separate lifecycle observation. Instrumentation can change interleaving even when aggregate throughput appears close.

Native and io_uring package tests passed (see tests.log), including existing parallel-dispatch coverage. The earlier source-counter experiment and original baseline matrices remain untouched. No commits were pushed.
