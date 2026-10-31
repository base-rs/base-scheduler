# Worker-local source interrupt counters

Experimental source counters on exact prototype `4a061fbc` and direct-read variant `6bd8719`. Counters use thread-local Cell<u64> storage with no contended atomics, and print only at thread teardown. The one-worker benchmark marks notifications issued from the installed worker thread as local; notifications from every other thread are remote. This is equivalent to the target worker in this one-worker workload; do not generalize that classification to multiple worker/controller groups.

Three randomized repetitions per executable; one worker, one connection, 64 bytes; 1-second warmup and 3-second measurement. Frozen controls are the exact previously retained binaries. Counter builds use `cargo build --release --locked` with `CARGO_PROFILE_RELEASE_DEBUG=1`, RUSTFLAGS unset. The older frozen direct-read executable originally used RUSTFLAGS=-Cdebuginfo=1; its hash is preserved. Shared build locks and exclusive timing locks were used.

| Variant | Median measured req/s | Median lifecycle voluntary CSW | Measured req/s samples |
| --- | ---: | ---: | --- |
| prototype-frozen | 220,754 | 887,177 | 220,754, 199,077, 223,692 |
| prototype-counted | 219,220 | 870,959 | 219,220, 227,445, 212,531 |
| direct-read-frozen | 189,884 | 750,051 | 190,473, 179,903, 189,884 |
| direct-read-counted | 193,995 | 766,333 | 195,280, 190,065, 193,995 |

prototype counter/frozen median measured throughput change: -0.69%.

direct-read counter/frozen median measured throughput change: +2.17%.

All counts below are full server lifecycle totals, including warmup and shutdown. `payload CQE pairs` is payload CQEs / 2 and approximates completed echo RTTs over the lifecycle; the final EOF receive contributes half a pair, so it is kept as an explicit .5 rather than confused with measured client requests. Untagged CQEs in this workload are original socket receive/send completions, including any unsuccessful attempts or partial transfers. This counter does not independently prove the absence of retries, so the denominator is an estimate rather than an exact successful-request count. A blocking-capable wait count is the number of wait calls, not proof that every call actually slept. Voluntary CSWs provide a separate scheduling observation.

| Variant / repeat | Measured client requests | Lifecycle payload CQEs | Lifecycle payload CQE pairs | Local notify writes | Remote notify writes | Notification SQEs / CQEs | Wait calls | Synchronous read success / EAGAIN | Voluntary CSW |
| --- | ---: | ---: | ---: | ---: | ---: | --- | ---: | --- | ---: |
| direct-read-counted / 1 | 581,985 | 1,532,891 | 766,445.5 | 749,903 | 2 | 749,820 / 749,820 | 766,447 | 0 / 0 | 766,333 |
| direct-read-counted / 2 | 585,839 | 1,566,669 | 783,334.5 | 766,403 | 2 | 766,334 / 766,334 | 783,336 | 0 / 0 | 783,232 |
| direct-read-counted / 3 | 570,196 | 1,529,811 | 764,905.5 | 748,401 | 2 | 748,315 / 748,315 | 764,907 | 0 / 0 | 764,787 |
| prototype-counted / 1 | 637,594 | 1,706,389 | 853,194.5 | 830,258 | 2 | 830,010 / 830,010 | 853,046 | 830,071 / 1,683,117 | 852,883 |
| prototype-counted / 2 | 657,661 | 1,742,523 | 871,261.5 | 847,816 | 2 | 847,615 / 847,615 | 871,137 | 847,669 / 1,718,806 | 870,959 |
| prototype-counted / 3 | 682,336 | 1,800,893 | 900,446.5 | 876,225 | 2 | 876,021 / 876,021 | 900,314 | 876,093 / 1,776,407 | 900,163 |

| Variant | Notification CQEs / lifecycle RTT estimate | Local writes / lifecycle RTT estimate | Wait calls / lifecycle RTT estimate | Voluntary CSW / lifecycle RTT estimate |
| --- | --- | --- | --- | --- |
| prototype-counted | 0.9728–0.9729 | 0.9731–0.9731 | 0.9998–0.9999 | 0.9996–0.9997 |
| direct-read-counted | 0.9783–0.9783 | 0.9784–0.9784 | 1.0000–1.0000 | 0.9998–0.9999 |

The interrupt path is exercised frequently in both instrumented implementations. Almost every notification write is from the same worker thread; each run has only two successful remote-thread writes. Every eventfd write succeeds: there are no EAGAIN or other notification-write errors. The PollAdd implementation additionally performs synchronous eventfd reads; direct Read consumes the event counter through its CQE and performs none.

Do not equate notification CQE frequency with external interrupts or with actual parking. These notifications are predominantly same-worker task wakeups. The source counters show roughly one interrupt CQE per lifecycle RTT in this run, whereas the prior BPF run showed a smaller PollAdd fraction and changed throughput materially. The frequency is timing-sensitive, and instrumentation can change the event-loop interleaving even if aggregate throughput stays close. The robust finding is substantial interrupt activity and overwhelmingly local signalling, not a precise universal uninstrumented CQE/RTT ratio.

The untouched original 300-sample matrix already provides the strongest uninstrumented parking evidence: voluntary context-switch totals rose substantially in the regressing prototype. These source counters are a mechanism/frequency check, not replacements for that performance matrix.

No optimizations or shipping code changes were made. The instrumentation commits, patches, hashes, frozen binaries, runner, and raw stderr/JSON are retained here.
