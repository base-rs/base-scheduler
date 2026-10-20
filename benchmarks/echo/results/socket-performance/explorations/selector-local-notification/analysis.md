# Selector-local notification flag experiment

The plain runtime source is 46f11468be79f4bf46ff8171f7b66498d8405e56: base 63da755 plus only io_uring selector/worker changes. Native WorkerState::wake is unchanged. Controller::Wake sets a matching controller LocalRoute Cell<bool> in TLS; otherwise it calls existing notify. Controller::send_remote is unchanged and always uses the pipe. Reactor consumes the local flag at the existing pipe-drain point, then drains the remote pipe in the same pass before deciding whether to return or wait.

Three randomized samples each of frozen selected and plain local-flag builds, then a separate three-sample counted comparison. One worker, one connection, 64 bytes, 1 s warmup plus 3 s measurement. Source counters were not used in the plain performance comparison. Builds held the shared /tmp/core-scheduler-perf.lock and each timing matrix held it exclusively.

| Matrix / variant | Median measured req/s | Median lifecycle voluntary CSW | req/s samples |
|---|---:|---:|---|
| plain/selected-frozen | 237,924 | 89,000 | 240,279, 237,924, 234,969 |
| plain/selected-local-notify | 179,447 | 719,886 | 179,175, 179,447, 180,713 |
| counted/selected-frozen-counted | 235,021 | 140,796 | 235,021, 234,274, 238,299 |
| counted/selected-local-notify-counted | 182,237 | 732,890 | 176,125, 185,652, 182,237 |

Plain local flag median throughput change versus frozen: **-24.58%**. This fails the performance screen; no larger workload matrix was run.

| Counter variant/repeat | Same-worker pipe writes | Other-thread pipe writes | Local flag set/taken | Wait syscalls | Voluntary CSW |
|---|---:|---:|---|---:|---:|
| selected-frozen-counted/1 | 915,672 | 2 | 0/0 | 940,090 | 140,796 |
| selected-frozen-counted/2 | 917,306 | 2 | 0/0 | 942,302 | 135,038 |
| selected-frozen-counted/3 | 913,534 | 2 | 0/0 | 938,212 | 146,119 |
| selected-local-notify-counted/1 | 0 | 2 | 717,271/717,239 | 737,149 | 736,865 |
| selected-local-notify-counted/2 | 0 | 2 | 683,612/683,567 | 702,536 | 702,256 |
| selected-local-notify-counted/3 | 0 | 2 | 713,497/713,441 | 733,246 | 732,890 |

The local flag collapses same-worker pipe writes to zero while preserving the remote writes. The flag is exercised and consumed frequently. Nevertheless actual voluntary scheduling switches rise substantially in the plain and counted comparisons. This is consistent with the removed notification write/read syscalls having supplied enough delay or extra loop opportunity for networking completions to arrive before entering a blocking wait. This is an interpretation of the timing and context-switch evidence, not direct proof of exact kernel interleaving.

All counts and context switches cover server lifecycle including warmup and shutdown. Throughput covers the measured client window. Untagged payload CQEs / 2 remain a lifecycle estimate, including any retries, partial transfers and final EOF, rather than a measured request count. Same-worker classification is valid for this one-worker workload.

No code was pushed. No coverage or additional runtime optimizations were run. Production tests for the same candidate are being implemented and validated independently by the parent agent. This isolated worktree contains no native worker suppression changes.
