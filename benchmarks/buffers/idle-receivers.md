# Idle receivers and shared capacity

A demand-driven receiver should not retain a scarce shared buffer while its socket has no data. The portable receiver now acquires storage, polls its nonblocking message receive once, and returns the storage if that attempt is pending. It waits for advisory monitor readiness without a buffer, then reacquires storage and retries. Regression and shared conformance tests exercise a one-slot pool with an idle receiver alongside an active receiver, readiness retries, empty datagrams, stream EOF, cancellation, and retained views.

This depends on an explicit borrowed-message contract: the first poll attempts nonblocking I/O after synchronous preflight, and any consumed message must complete in that poll. Returning `Pending` consumes nothing, so dropping that attempt cannot lose a datagram. Scalar reads retain their separate contract and readiness-first optimizations.

## Removing redundant Tokio probes

The first implementation introduced a small-message Tokio regression. An initial [448-sample comparison](results/idle-receivers-initial.jsonl) found median matched throughput changes of −4.5% and −5.6% for pooled TCP with 64-byte messages, a window of one, and one and two workers respectively. Other cases were mostly close, so an aggregate alone would have hidden this problem.

Native `poll` probes in Tokio's socket wait were unnecessary once the socket already had a Tokio readiness registration. The new receiver exercised that wait explicitly after returning its buffer. A syscall trace of the one-worker case, with 3,000 timed exchanges plus 1,000 warmup exchanges, showed:

| Implementation | Native `poll` calls | `recvmsg` calls |
| --- | ---: | ---: |
| Before idle buffer release | 2 | 7,972 |
| Initial idle buffer release | 2,406 | 7,962 |
| Reuse registered Tokio readiness | 2 | 7,963 |

Registered Tokio waits now use the existing readiness state and directional waiter reservation directly. Actual receives still attempt their nonblocking syscall and clear stale readiness on `WouldBlock`. Unregistered sockets retain syscall-first handling to avoid permanently caching an unconnected stream's hangup, and Windows retains its existing path. The trace is diagnostic: tracing changes scheduling, so its timings are not performance measurements.

## Optimized comparison

The final [448 samples](results/idle-receivers.jsonl) compare baseline `2b225331f54e1d61b9bcb56916717d71a230ea02` with the production behavior in `068ae534a86d0ab7f774e25e490aefcac33df1e4`. The initial candidate corresponds to `eecfd57efd77e81f52184eac7021b746f64324dd`. Binaries were built from worktree snapshots during development; subsequent comment/import formatting and report edits do not change their production logic. Every sample records the actual binary SHA-256:

- Baseline: `1740856c28f32bc47db8a2088fc802acf12cc47bfd303c8dc4f740d25dd1dba1`.
- Initial candidate: `c1e6fc24d6ee8d3fdff29ca0968ac4551aa73add15ab8a89a54c00947b87fb04`.
- Final candidate: `dc3b9787137274378b06d444bed7b273c41d2c3b52068f19f00236b5b633f213`.

Hana has an Intel Xeon E3-1240 v6, four physical cores with eight hardware threads, running Linux 7.2.3-arch1-2. Both binaries use Cargo's optimized release profile. Timing runs hold `/tmp/core-scheduler-perf.lock`, with no overlapping builds, tests, or other cooperating benchmarks. Each process has CPU affinity 2–5, without individual thread pinning.

There are seven repetitions of each backend, worker count, receive mode, and workload. Case order and paired baseline/candidate order are randomized with seed 232810. Each sample uses one connection, eight pool slots where applicable, 1,000 warmup batches, and 3,000 measured batches. `raw` is the borrowed monitor control; `pool` includes buffer acquisition and return. These are loopback receive workloads, without parsing, encryption, physical-network traffic, or io_uring worker-provided buffers.

The table gives median changes in messages per second using matched candidate/baseline ratios within each repetition:

| Workload / window | Workers | io_uring pool | io_uring raw | Tokio pool | Tokio raw |
| --- | ---: | ---: | ---: | ---: | ---: |
| TCP 64 B / 1 | 1 | +0.3% | +0.6% | −3.0% | +1.3% |
| TCP 64 B / 1 | 2 | −1.3% | +2.5% | +0.5% | −0.1% |
| TCP 64 B / 16 | 1 | −0.3% | −0.9% | −1.4% | −1.1% |
| TCP 64 B / 16 | 2 | −0.1% | −0.2% | +1.4% | +1.4% |
| TCP 64 KiB / 4 | 1 | −0.3% | +0.6% | +0.6% | +0.7% |
| TCP 64 KiB / 4 | 2 | +0.7% | −0.9% | −0.4% | +0.8% |
| UDP 1,200 B / 16 | 1 | +1.4% | +1.1% | −1.2% | −6.8% |
| UDP 1,200 B / 16 | 2 | +0.1% | −1.1% | −1.4% | +0.5% |

Across all matched repetitions, the geometric mean changes are −0.06% for io_uring pool, +0.10% for io_uring raw, −0.19% for Tokio pool, and +0.18% for Tokio raw. These aggregates do not establish equivalence: the one-worker Tokio small-TCP median remains −3.0%, while its individual ratios range from −11.6% to +10.2%. The raw UDP control also varies substantially.

### Longer one-worker Tokio check

A further [168 samples](results/idle-receivers-focused.jsonl) reuse the final binaries, affinity, and locking, with 21 randomized matched repetitions and 15,000 timed batches per sample. This narrows the comparison to the small TCP and UDP cases whose short-run medians were most concerning:

| Workload / window | Mode | Median change | Repeat range |
| --- | --- | ---: | ---: |
| TCP 64 B / 1 | Pool | −0.8% | −4.4% to +2.2% |
| TCP 64 B / 1 | Raw | −0.1% | −4.3% to +2.6% |
| UDP 1,200 B / 16 | Pool | −0.2% | −2.9% to +0.9% |
| UDP 1,200 B / 16 | Raw | −0.2% | −1.7% to +2.1% |

The larger regression from redundant native readiness probes is no longer present. Releasing and reacquiring idle storage still performs extra pool bookkeeping; these results do not prove that cost is zero. They support keeping bufferless idle waits for shared-capacity correctness, without claiming a speedup. Workloads with many idle connections, parser-retained views, or different CPU placement need their own measurements.

## Reproduce

Build each revision separately and preserve its executable:

```sh
cargo build --release --locked -p core-scheduler-buffer-benchmark --bin sockets
```

The runner refuses to overwrite an existing output file. It records binary hashes and checks each benchmark subprocess's result:

```sh
flock /tmp/core-scheduler-perf.lock python3 benchmarks/buffers/idle_receivers.py \
  --before /path/to/sockets-baseline --after /path/to/sockets-candidate \
  --output /tmp/idle-receivers.jsonl
```

For the longer focused check, use the same command with a fresh output path and `--repeats 21 --iterations 15000 --backends tokio --workers 1 --workloads tcp-small udp`.

For syscall diagnosis, run the following separately for each binary and compare call counts:

```sh
flock /tmp/core-scheduler-perf.lock strace -f -c -e trace=poll,epoll_wait,epoll_ctl,recvmsg \
  taskset -c 2-5 /path/to/sockets tokio pool 64 3000 1 tcp 1 receive
```
