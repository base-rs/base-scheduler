# Socket Stream Immediate Reads

## Requirement

`SocketStreamReader::try_read_into` is a required, synchronous method that attempts one borrowed read now. Every backend implementation must reach the transport on each call: it must not wait, register interest, or return `WouldBlock` because of cached readiness. It reports input, EOF and errors the operating system has already queued. `WouldBlock` means no input is available now, or an abandoned owned operation still retains the handle. A closed reader returns `BrokenPipe`; only the reader's own handle is checked, so closing the scheduler does not stop immediate reads. It does not detect urgent data. Zero with nonempty storage is EOF; an empty destination returns zero without establishing EOF. The method must not change readiness-first behaviour of the asynchronous reads.

Wrapping readers forward it to their inner reader. The method has no default, so wrappers cannot silently fall back to a weaker probe.

## Rationale

base-http probes an idle pooled connection by polling one read with a no-op waker before reusing it. That only observed a server's close when the poll performed a syscall. Native and Tokio readers that have established progress wait on cached readiness first, so the probe missed closes until the owning worker next turned its driver, and the next request failed on a dead connection ([base-rs/base-http#42](https://github.com/base-rs/base-http/pull/42)). The maintainer chose an explicit immediate read over making every established read syscall-first, which would undo the readiness optimisation, and chose a required method over a defaulted one so that wrappers must forward it.

## Scope

The `SocketStreamReader` interface and its backend implementations: the readiness selector (used by Native), io_uring, the Tokio adapter and Direct. Packet readers, pipes and files are outside this requirement.

## Validation

Run `socket_receiving::stream_try_read` from the conformance crate against each backend: an empty destination, `WouldBlock` with no input, queued input after an established short read, EOF after the peer shuts down writing, and `BrokenPipe` after close. Bound-I/O tests check `WouldBlock` while an abandoned owned read retains the handle. On Unix Native, where established reads wait on cached readiness, `immediate_reads_observe_input_while_the_only_worker_is_busy` keeps the only worker busy, so its driver cannot publish readiness: a single poll of `read_into` stays pending while `try_read_into` returns the queued byte.
