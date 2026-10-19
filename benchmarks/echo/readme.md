# TCP echo benchmark

Compare pure Tokio, `TokioScheduler`, and `NativeScheduler<IoUringSelector>` using the same persistent-connection TCP echo workload. Both scheduler adapters offer owned-buffer and borrowed `_poll` cases. The private workspace package adds no dependencies to the public scheduler interface.

The [initial Aiko results](results.md) include throughput, latency, variation across repetitions, and raw samples.

The [performance investigation](profiling.md) compares compiler settings, identifies CPU and syscall costs, and measures isolated optimization prototypes.

The [io_uring performance improvements](performance.md) describe the implemented fast paths, their relationship to `io-event`, and repeated before/after measurements.

The [borrowed socket comparison](poll-performance.md) measures both adapters' `_poll` paths alongside their owned operations and pure Tokio, and identifies opportunities in the native readiness integration.

Build and run the comparison on Linux:

```sh
python3 benchmarks/echo/run.py --server-cpus 0-3 --client-cpus 4-7 --output /tmp/echo-results.json
```

Check `lscpu -e=CPU,CORE,SOCKET,NODE,CACHE` first: those CPU numbers must refer to separate physical cores, preferably sharing one last-level cache. The runner uses the first `workers` entries from the server CPU set. CPU affinity is optional; omit both options on platforms without `taskset`. io_uring requires Linux; use `--backends tokio tokio-adapter tokio-adapter-poll` elsewhere.

The default matrix compares 1 and 4 server workers, 1 and 32 connections, and 64-byte and 4 KiB payloads. Each case runs three times with a one-second warmup and three-second measurement. Case and backend order are shuffled with a fixed seed. Adjust `--workers`, `--connections`, `--bytes`, `--repeats`, `--warmup`, and `--seconds` for longer or different workloads. The client has four Tokio workers by default (`--client-workers`).

The runner builds an optimized binary using the checked-in lockfile, then starts separate server and client processes for every sample. Pass `--client-binary /path/to/baseline` to keep an existing client executable fixed across server changes. Raw results record both binary hashes. Servers bind an ephemeral IPv4 loopback port, accept the requested number of connections, and exit after all clients close. Both peers enable `TCP_NODELAY`. Server buffers are 64 KiB and allocated once per connection; writes handle short transfers.

| Backend | Executor | Socket payload I/O |
| --- | --- | --- |
| `tokio` | Tokio | `TcpStream::read` / `write_all` with borrowed buffers |
| `tokio-adapter` | Tokio | Owned `socket_read` / `socket_write`, implemented with readiness |
| `tokio-adapter-poll` | Tokio | Borrowed `socket_recv_poll` / `socket_send_poll`, implemented with readiness |
| `iouring` | Native | Owned `socket_read` / `socket_write`, submitting io_uring `Recv` / `Send` |
| `iouring-poll` | Native | Borrowed `socket_recv_poll` / `socket_send_poll`, using direct syscalls and ring readiness waits |

The adapters share one generic owned echo loop and one generic borrowed echo loop. The borrowed loop uses default send/receive flags and keeps the buffer in its connection future. Ready calls can finish in the same task poll; calls returning `WouldBlock` wait for readiness. The io_uring configurations both retain one ring per worker, 256 SQ entries per ring, and admission capacity of at least the connection count. `iouring-poll` does not submit payload transfers to those rings; it submits `PollAdd` readiness waits only when a syscall would block. Listener handling uses the readiness fallback in both cases. Comparing owned and borrowed modes therefore measures a change of I/O mechanism for the native backend, while both Tokio adapter modes already use readiness. The historical borrowed-socket report records the earlier shared-readiness fallback implementation.

The client connects and allocates buffers and histograms before warmup. All connections use one shared measurement window. Each sends one complete payload, waits for its full echo, verifies the bytes, and then sends the next request. Only round trips entirely inside the measurement window are counted. Neither connection setup nor warmup contributes to the reported throughput or latency histogram. One request means one round trip; payload MiB/s counts bytes in one direction, so aggregate TCP payload traffic is twice that figure.

Latency is round-trip latency as seen by this closed-loop client, with three significant digits in the HDR histogram. It includes client scheduling and loopback networking. This is not an open-loop arrival-rate test, does not correct for coordinated omission, and does not measure connection churn or pipelining. The comparison measures these concrete implementations and ownership paths, not just trait-dispatch overhead. A client bottleneck can limit every server; verify client headroom before treating throughput as server capacity. Loopback results do not predict physical-network performance.

Raw JSON preserves every repetition, configuration, CPU topology, source revision, toolchain, and load averages. Compare medians and variation across repetitions rather than selecting the fastest run. Keep frequency policy and affinity consistent; the runner does not change machine-wide settings.

To run the binaries directly:

```sh
cargo build --release --locked -p core-scheduler-echo-benchmark
# Prints an address; remains running until the four clients close:
target/release/core-scheduler-echo-benchmark server iouring 4 4
# Substitute the printed address:
target/release/core-scheduler-echo-benchmark client 127.0.0.1:PORT 4 4 64 1 3
```

Smoke tests exercise each available backend, payload validation, invalid inputs, and empty measurement windows. A controlled socket fixture verifies partial writes, zero-progress errors, and buffer reuse:

```sh
cargo test --locked -p core-scheduler-echo-benchmark
```
