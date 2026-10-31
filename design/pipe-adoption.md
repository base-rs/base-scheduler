# Pipe Adoption

## Requirement

On Unix, applications can adopt an owned pipe descriptor, such as a child process's standard input or output, as a sequential asynchronous reader or writer on every backend. `PipeReading::pipe_reader` and `PipeWriting::pipe_writer` take ownership of an `OwnedFd`, set it nonblocking, and return a handle bound to the scheduler without borrowing any temporary reference.

Readers report end of file as a zero-length read once every write end is closed. Writers report a closed read end as an `io::ErrorKind::BrokenPipe` error, which requires `SIGPIPE` to be ignored as it is by default in Rust programs. Empty transfers complete immediately without I/O. Operations support cooperative cancellation. `close().await` retires the handle and closes the descriptor, reporting the close result; later operations fail with `BrokenPipe` and repeated closes succeed. Dropping a handle closes the descriptor. Construction failure closes the descriptor.

## Rationale

Tomori's server runs agent, speech recognition and synthesis workers as child processes and exchanges line-delimited messages with them over standard input and output. The standard library spawns the processes and exposes `ChildStdin` and `ChildStdout`, which convert into `OwnedFd`. Without an adoption capability the application either blocks a scheduler thread or uses a separate runtime for these pipes.

## Scope

The `PipeReading`, `PipeWriting`, `PipeReader` and `PipeWriter` traits; Direct, readiness selector, Tokio, Native forwarding, and io_uring through its readiness fallback; and reusable conformance assertions. Owned-buffer reads and writes, FIFOs, regular files, terminals, and Windows anonymous pipes are out of scope. Linux and macOS are validated; other Unix platforms may reject readiness registration for the opposite direction of a pipe end. Setting `O_NONBLOCK` affects the shared file description, including other descriptors duplicated from it.

## Validation

Round-trip data through a real `cat` child on every Unix backend, including concurrent reads and writes larger than the pipe buffer. Exercise vectored transfers, end of file, empty transfers, broken pipes, cancellation of blocked operations, and close semantics including operations after close. Run the workspace tests and coverage on macOS and Linux, including io_uring.
