# I/O recipes

These executable examples share their loops with correctness tests and a standard benchmark driver. They keep schedulers and cancellation explicit, reuse monitors and storage across operations, and await cleanup after both success and failure. They are a private workspace package, with no concrete backend dependencies added to the published interface crate.

| Recipe | Core loop | Ownership |
| --- | --- | --- |
| `stream` | [`stream::receive`](src/stream.rs) | Process a borrowed chunk before reusing storage. |
| `frames` | [`frames::receive`](src/frames.rs) | Retain immutable views of complete fixed-size records; copy a record only when it spans receives. |
| `datagrams` | [`datagrams::receive`](src/datagrams.rs) | Transfer whole messages and peer metadata to bounded worker tasks. |
| `files` | [`files::copy`](src/files.rs) | Alternate two owned buffers while joining the next read with the current write. |

The frame decoder is deliberately a small fixed-record byte-stream example, not an HTTP implementation or a datagram parser. The driver retains two frames over later receives and checks their contents again during processing and after receiver close. The UDP loop bounds task ownership and waits for every accepted task even after an error. It collects handler results in submission order: an error is observed at collection, so callers must cancel a receive wait when ending a sequence early. Handlers must finish independently of further receives. Applications retaining data beyond these handlers must budget for that additional ownership.

Run from the workspace root:

```sh
cargo run --release -p core-scheduler-examples -- native stream 1024 16384 1
cargo run --release -p core-scheduler-examples -- tokio frames 257 65536 2
cargo run --release -p core-scheduler-examples -- direct datagrams 1200 10000 2
cargo run --release -p core-scheduler-examples -- iouring files 65536 1024 2
```

Arguments are `backend recipe record-bytes record-count workers [receive-buffer-bytes]`. Backends are `direct`, `tokio`, `native` (readiness), and Linux-only `iouring`. Storage defaults to 64 KiB. Datagram records must fit both UDP and the receive storage and include an eight-byte sequence number. The driver emits one JSON sample with throughput and workload parameters.

## Measurement boundaries

Every invocation runs one complete warmup and one measured pass on a spawned scheduler task. Sockets, files, payloads, and storage are prepared before timing. Timing includes the recipe's one monitor/receiver construction and close, so lifecycle overhead is amortized across the complete transfer. Exclusive resource close and peer join happen after timing. Frames and network bytes are validated during processing; file output is checked afterwards. File pages are prepopulated: these are page-cache copies, without durability synchronization.

TCP has one sender thread and one scheduler-side receive sequence. UDP has a window of eight packets and eight processing tasks; acknowledgements over a host channel bound the generator so packet loss is not mistaken for throughput. Packet validation and completion notifications are included. The frame pool has sixteen slots and at most two retained frame owners. No benchmark promises worker migration or uses worker pinning.

Compare each recipe across backends. Different recipes perform different application work: the frame and packet cases deliberately pay for ownership and processing. Their throughput is not a direct comparison of pooled versus borrowed syscalls. Existing mechanism comparisons remain in the [buffer benchmarks](../benchmarks/buffers/readme.md).

Run the standard matrix with:

```sh
python3 examples/benchmark.py --repeats 5 --output /tmp/core-scheduler-examples.jsonl
```

The script builds once, obtains the executable path from Cargo (including custom target directories), and shuffles each repetition with a fixed seed. It records JSON samples and a neighboring metadata file with compiler, platform, source revision, and workload provenance. See [results](results.md) for the measured baseline and machine details.

The installed [I/O context guide](../context/io.md) records the recommendations for protocol authors and agents. The [interface reference](../context/interface.md) describes the complete capabilities and ownership contracts.
