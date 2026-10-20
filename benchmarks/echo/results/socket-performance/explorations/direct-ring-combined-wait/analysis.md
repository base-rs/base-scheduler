# Combined submission/wait experiment

Idle: zero CPU ticks across owned/borrowed × one/four workers.

Serial 1w/1c/64B, randomized three repeats; median requests/s:

| Backend | Original control | Ring-retry | Combined | Combined vs original |
|---|---:|---:|---:|---:|
| iouring | 235,713 | 189,846 | 185,792 | -21.18% |
| iouring-poll | 181,635 | 214,119 | 226,915 | +24.93% |

The owned regression persists. Do not ship this runtime solely from borrowed gains. Next: synchronous nonblocking send before owned request allocation, retaining existing completion path on WouldBlock.
