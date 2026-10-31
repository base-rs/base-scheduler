# Final Tokio syscall comparison

Three focused traces ran after the final matrix released the exclusive host lock, on Aiko from 2026-10-20T00:31:30Z to 2026-10-20T00:31:32Z. The exclusive lock remained held through trace export. No code changes, builds, or additional timing samples were made.

Server source: `938e7e641f8d186991a0f46f211f4ce3d19c929e`; executable SHA-256 `f5f0336a61e7ccfbf6db35b1ac10cf04845137465ef4250c0420691b1173908e`. Fixed client source: `5736301fded46c4e5f11d5387950ae2ee0807b94`; executable SHA-256 `db6c914ab75011477ec6232e681046b1e75532cbaa1c81663bac984591675cf3`. Both hashes were checked before any trace.

Each sample: one worker on CPU 0, 32 connections, 64-byte messages, client CPUs 4–7, no warmup, 0.5-second measurement. `strace -f -C -s 0` captured data syscalls plus accept/close. The parser tracks accepted socket descriptors and excludes loader, stdout, notifier, and other unrelated reads/writes. Exact EAGAIN errno counts come from raw syscall returns; summary error counts alone were not used. Raw traces are retained compressed beside short `summary.strace` and parsed `syscalls.json` files.

| Backend | Measured complete requests | Socket recv calls | Recv EAGAIN | Socket send calls | Send EAGAIN | Recv calls/measured request | Recv EAGAIN/measured request |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| tokio | 28,402 | 28,466 | 0 | 28,434 | 0 | 1.002253 | 0.000000 |
| tokio-adapter | 40,083 | 40,150 | 3 | 40,115 | 0 | 1.001672 | 0.000075 |
| tokio-adapter-poll | 31,950 | 32,030 | 16 | 31,982 | 0 | 1.002504 | 0.000501 |

All sends succeed. All successful data receives and sends transfer exactly 64 bytes. Each trace additionally includes 32 final boundary-crossing round trips and 32 EOF receives. The client counts only requests wholly within its measurement window, so these lifecycle counts are deliberately distinguished from the requested per-measured-completion normalization. There are no other accepted-socket I/O errors.

Observed receive EAGAIN is **0.000%, 0.0075%, and 0.0501% per measured request** for pure Tokio, owned adapter, and poll adapter respectively. These traces do not demonstrate a material excess of failed receive attempts. They cannot establish how much the syscall-first retry contributes to the uninstrumented 1-worker throughput gap: ptrace changes scheduling and readiness timing, and almost every receive succeeded immediately in this traced workload. Traced throughput is intentionally omitted as capacity evidence.

Source-level distinction remains: Tokio 1.53.3 `io/poll_evented.rs::poll_read` waits for cached readiness before reading and, on Linux, clears that readiness after a successful short read (`0 < n && n < len`). The benchmark receives 64-byte messages into a 65,536-byte buffer, so this optimization applies. The adapter attempts a syscall before readiness gating and only clears cached readiness after a retry returns WouldBlock. That structural difference can allow an additional failed attempt when cached readiness is stale; these perturbed traces do not quantify its normal frequency.

Do not attribute the remaining measured gap to duplicate recv calls based on these three traces. Quantifying that claim would require a counter with substantially less scheduling perturbation.
