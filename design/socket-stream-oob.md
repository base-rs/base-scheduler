# Socket Stream Urgent-Data Policy

## Requirement

Socket stream readers default to OOB support disabled. Applications can select support at compile time with `reader.with_oob_support::<true>()`, without changing scheduler factories or introducing a runtime option branch in the specialized receive path.

Selecting either policy consumes the reader and preserves its scheduler binding, socket, registration, receive pool, and established-read, EOF and close state. It neither initiates I/O nor creates another task or receive queue. Cancellation and resource-retirement contracts remain unchanged.

Enabled support must preserve ordinary-byte progress across TCP urgent marks, including when a socket alias consumes the urgent byte. The option does not receive the urgent byte itself. Disabled support permits an `Unsupported` error for an observed urgent condition, and applications must not consume urgent data through aliases in that mode. Disabled support is not a network filter or a guarantee that every urgent byte is detected. Backends may retain a conservative implementation for both settings.

On native Linux epoll and io_uring, disabled scalar borrowed reads may clear their observed readiness after a positive short receive when no closure condition has been reported. They must preserve a newer notification racing that clear. Reported EOF and urgent conditions remain sticky across ordinary readiness clears. Enabled readers retain readiness until `WouldBlock` to preserve progress across urgent marks. Compatibility polling without directional EOF information retains conservative reads. On io_uring, mixed input/priority wake hints are advisory; only isolated priority notifications establish an urgent condition for the disabled scalar reader. Owned, vectored and flagged receives retain their existing behavior.

macOS retains its existing byte-count and EOF policy for both settings. A short read may clear its observed readiness only after consuming at least the event's reported available bytes and when EOF was not reported.

## Rationale

The HTTP performance investigation identified a redundant receive after ordinary short Linux TCP reads. The user requested a compile-time OOB option, disabled by default, so the common scalar path can avoid that receive while applications needing urgent-mark progress can select conservative handling. Platform-specific event information limits where the optimization is valid.

## Scope

The public `SocketStreamReader` adapter and backend stream-reader implementations, including conversion of live readers between policies. Packet receives and delivery of the TCP urgent byte are outside this requirement.

## Validation

Inspect the public adapter, const-generic readiness implementation, and platform event dispatch. Exercise ordinary short reads, EOF with final data, notification-generation races, cancellation, close, receive-pool retention and live-reader policy changes. On Linux and macOS, exercise normal bytes before and after an urgent mark with enabled support, both with and without a socket alias consuming the urgent byte. On native Linux epoll, verify that an observed urgent condition rejects a disabled reader and that selecting enabled support preserves the remaining normal bytes. On io_uring, exercise disabled short-read clearing with notification-generation races and sticky closure, rejection of isolated priority notifications, ordinary input with mixed priority hints, and conservative enabled reads across urgent marks. Pooled and message receive paths retain their existing conservative behavior.

Compare optimized-build HTTP and raw socket workloads against a preserved baseline with matched worker counts, response bytes, connection reuse and task context. Record both throughput and process CPU per response, including low-concurrency tradeoffs. Performance measurements support the implementation choice; no numerical acceptance threshold is established by this requirement.
