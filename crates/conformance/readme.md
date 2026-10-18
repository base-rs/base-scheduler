# Protocol Scheduler Conformance

Reusable, capability-specific tests for implementations of `protocol-scheduler`. Add this crate as a **development dependency** of a backend. Generated suites report ordinary named Rust tests, while async assertion helpers remain available for custom fixtures and smaller capability sets.

## Generated suites

Invoke a suite inside a dedicated module and supply a runner function:

```rust,ignore
mod blocking {
    protocol_scheduler_conformance::blocking_tests!(run: super::run_blocking);
}

mod ipv4 {
    protocol_scheduler_conformance::socket_tests!(run: super::run_ipv4_stream);
}
```

The conformance crate owns each suite's case list. Updating this dependency automatically includes new cases in every backend using that suite; backends do not maintain copies of test names. Each expansion produces separate `#[test]` functions such as `blocking::pre_cancelled_submission` and `ipv4::pending_read_cancellation`. Normal `cargo test` listing, filtering, parallel execution, and failure reporting work without a custom test harness. A failed assertion does not prevent sibling cases from running.

A runner receives one async closure and creates a **fresh isolated fixture for each invocation**. For a blocking implementation named `Backend`, its signature is `fn run_blocking(test: impl AsyncFnOnce(&Backend))`. Stream and datagram runners accept `impl AsyncFnOnce(&Backend, &BackendSocket, &BackendSocket)`, supplying the scheduler, sender, and receiver by reference. Fixture runners accept `impl AsyncFnOnce(&Fixture)`. Async closures require Rust 1.85, matching the workspace minimum. Runners may add `Send + 'static` when moving the closure to a deadline-controlled thread; operation futures need not be boxed.

| Macro | Runner supplies | Required capabilities or fixture |
| --- | --- | --- |
| `blocking_tests!` | Scheduler | `Blocking` |
| `blocking_queue_tests!` | Fixture | `blocking::BlockingFixture` |
| `file_tests!` | Fixture | `FileTestFixture` with the complete `FileScheduler` profile |
| `socket_tests!` | Scheduler and fresh connected stream pair | `SocketRead`, `SocketWrite`, `SocketWait`, `SocketShutdown` |
| `socket_message_tests!` | Scheduler and fresh connected stream pair | `SocketSendMsg`, `SocketRecvMsg` |
| `socket_datagram_tests!` | Scheduler and fresh bound, unconnected datagram pair | `SocketSendMsg`, `SocketRecvMsg`, `SocketLocalAddress` |
| `socket_io_tests!` | Fixture | `SocketIoFixture` |

Run socket suites separately for each supported address family and the I/O lifecycle suite for each supported operation/input representation. Use individual assertion helpers for capability combinations smaller than a suite. Unsupported capabilities must not be presented as passing tests.

The runner drives its executor and services, awaits the supplied closure exactly once, enforces a deadline covering setup, assertions, and cleanup, and propagates assertion panics to the generated test. Cleanup must also run after assertion failure: release held execution boundaries, let accepted operations retire, and await backend shutdown before destroying temporary storage. Do not catch and discard a panic. The [direct file/blocking runners](../direct/tests/conformance.rs) and [socket runners](../direct/tests/sockets.rs) provide complete integrations.

## Assertions and fixtures

File helpers accept backend-created resources and paths and require only the capabilities they exercise. `FileTestFixture` additionally supplies independent backing-store observations, so the complete file suite checks contents without relying on the same scheduler implementation to verify its own writes. Each file case starts with an absent path, creates and seeds its own regular file, and closes it after its assertions.

Socket assertions cover separate creation/binding/listening/connection/acceptance, scalar and prepared message I/O, short transfers, EOF, readiness, IPv4/IPv6 addresses, datagram boundaries, peek, truncation, empty datagrams, cancellation, and ownership. `socket::stream_pair` creates an isolated loopback fixture; individual I/O assertions also accept backend-created sockets. `SocketIoFixture` and `SocketIoGuard` expose a backend-controlled access boundary for deterministic completion/cancellation races and buffer retention after waiter abandonment. Fixtures must release blocked access on destruction. Blocking queue fixtures hold execution at a known boundary to check cancellation, admission, and handle abandonment without assuming global FIFO order.

Tests panic on contract violations. Existing aggregate helpers remain available. No test assumes one runtime, task order, concrete descriptor, or prompt cancellation of a running syscall. Scheduler shutdown, capacity/progress policy, platform-specific behavior, and unsupported capabilities still need backend-specific tests. Fixtures and assertions live here; backend-specific internal checks belong in backend crates. Future adapters can use the same suites without depending on direct.
