# Readiness-policy diagnostic.

This isolated screen changes only the pure Tokio receive loop, against production source 958ac6c. The workload is one server worker, 32 connections and 64-byte payloads, with three randomized repetitions per case, one second warmup and three seconds measurement.

| Receive loop | Median req/s | Range | Ratio to AsyncRead | Ratio to poll-ready |
| --- | ---: | ---: | ---: | ---: |
| after / tokio | 696,293 | 693,435–700,304 | 100.00% | 105.27% |
| readable / tokio | 649,123 | 648,113–652,898 | 93.23% | 98.14% |
| poll-ready / tokio | 661,454 | 660,271–663,029 | 95.00% | 100.00% |
| after / tokio-adapter | 641,955 | 638,558–643,395 | 92.20% | 97.05% |
| after / tokio-adapter-poll | 644,560 | 638,219–646,209 | 92.57% | 97.45% |

All 15 samples completed; sample stderr files are empty. Full raw samples are in screen/aiko.json and individual sample JSON files. Artifact fingerprints and limitations are in provenance.json.

The poll-ready variant uses poll_fn(TcpStream::poll_read_ready) plus try_read, preserving cached readiness until a syscall returns WouldBlock. It therefore follows the same readiness-retention policy as Core without importing Core wrappers. The readable variant uses readable().await plus try_read and is retained separately because it has a different waiter path.

Differences from the original AsyncReadExt::read loop cannot be assigned exclusively to EAGAIN: Tokio read-buffer handling, readiness APIs, cooperative budgeting and waiter registration also differ. This screen narrows the unexplained adapter overhead; three repetitions of one short echo workload do not establish universal parity or isolate syscall-level causes.

No production code changes, PRs, merges or releases were made by this diagnostic.
