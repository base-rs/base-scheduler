# Packet ECN

Accepted 2026-11-20; not yet implemented.

## Requirement

Packet sockets can report the Explicit Congestion Notification (ECN) marking of each received datagram and set the marking of the datagrams they send, through typed socket options and receive metadata. Reporting is disabled by default. The backends keep no per-socket state for it: every packet receive passes a small fixed control buffer and reports the marking the system attached, making no additional system call, and gets read the kernel options, decided on 2026-11-20.

Types and options:

- The interface crate defines `Ecn` with the four codepoints of [RFC 3168 §5](https://www.rfc-editor.org/rfc/rfc3168.html#section-5): `NotEct` (`00`), `Ect1` (`01`), `Ect0` (`10`) and `Ce` (`11`), with conversions from and to the two ECN bits of the IP TOS or traffic class byte. `SocketRecvResult` keeps its existing derives (`Clone`, `Copy`, `Debug`, `Default`, `Eq`, `PartialEq`), so `Ecn` derives at least `Clone`, `Copy`, `Debug`, `Eq` and `PartialEq`.
- `ReceiveEcn`, a boolean `SocketOption`, enables reporting: `IP_RECVTOS` on an IPv4 socket and `IPV6_RECVTCLASS` on an IPv6 socket.
- `SendEcn`, a `SocketOption` whose value is `Ecn`, sets the ECN bits of `IP_TOS` on an IPv4 socket or `IPV6_TCLASS` on an IPv6 socket for every datagram the socket sends, reading the current byte first so the DSCP bits are preserved.
- Getting either option reads the kernel option. On a dual-stack socket it returns the IPv6 option's value (`IPV6_RECVTCLASS` or the ECN bits of `IPV6_TCLASS`).
- Dual-stack sockets, which are IPv6 sockets with `IPV6_V6ONLY` disabled: setting either option also configures the IPv4 side that the platform uses for IPv4-mapped traffic, and fails if any of these calls fails. On Linux, `ReceiveEcn` also sets `IP_RECVTOS` on the IPv6 socket, and the kernel then reports mapped datagrams as an `IPPROTO_IP` `IP_TOS` control message. `SendEcn` also sets the ECN bits of the socket's `IP_TOS`, which Linux uses for mapped destinations.
- A dual-stack set that fails part-way returns the error and leaves the options it already changed as the system holds them; receives and gets then reflect that kernel state.
- On macOS, a dual-stack socket uses the same calls as on Linux, decided on 2026-11-20: the IPv4 options are set alongside the IPv6 ones, and receives accept either control message form. The conformance suite's macOS dual-stack run establishes whether that works; if it does not, this requirement is revised to name what does, or to return `ErrorKind::Unsupported` there.
- On a stream socket, setting or getting either option returns `ErrorKind::InvalidInput`, since TCP manages ECN in the kernel. This takes precedence over `Unsupported`, on every backend and platform. For example, Linux keeps the kernel's own ECN bits in a TCP socket's `IP_TOS`, so `SendEcn` would otherwise be accepted and then ignored.
- The interface crate defines these types without socket-option syscalls, as it does for the existing options. Each backend implements `SocketSetOption` and `SocketGetOption` for them, and Native forwards its selector's implementations. A backend that cannot report or set ECN on a packet socket on a platform returns `ErrorKind::Unsupported` from setting or getting the option. It never accepts a setting it cannot honour.

Receive metadata:

- `SocketRecvResult` gains a public field `ecn: Option<Ecn>`. It stays an exhaustive struct with public fields, constructed with struct literals as now; its `Default` remains an empty receive, with `ecn` `None`.
- `SocketRecvMessage` gains a provided method `fn ecn(&self) -> Option<Ecn>` returning `None`. `OwnedSocketRecvMessage` returns its result's value, and every backend message type that can carry the marking overrides it. This includes `IoUringRecvMessage`, both for multishot messages and for messages it builds from a portable result, which keep that result's marking.
- `Some` is reported only when the system supplied the marking for that datagram, which it does while the kernel's reporting option is enabled, whether through `ReceiveEcn` or otherwise. Everything else is `None`: reporting disabled, a datagram for which the system supplied no marking, or a stream socket. Absence is never reported as `NotEct`. Enabling or disabling `ReceiveEcn` takes effect for every reader and receiver of the socket, whenever it was constructed.
- Every packet receive form reports it: `read`, `read_into`, `read_vectored`, their flagged forms including peeks, owned and pooled receives, and the io_uring multishot `recvmsg` experiment. The multishot experiment rejects peeks.
- Every packet receive passes control-message storage to the system with room for one marking message of either form: a one-byte IPv4 TOS or an `int` IPv6 traffic class, that is `CMSG_SPACE(sizeof(int))`. With reporting disabled the system attaches nothing to it. This replaces passing no control storage (`msg_controllen` zero). A larger control message the system attaches for an option set outside the backend, such as `IP_PKTINFO`, whose `in_pktinfo` needs more space than one `int`, still sets `control_truncated`.
- If the system truncated a receive's control data, the receive sets `control_truncated` and reports `None`. This applies even if the marking message itself arrived intact: another control message can displace it, and the result stays deterministic. Control messages other than the marking are ignored.

Flags:

- `SocketSendFlags` is unchanged. Per-datagram ECN on send is not part of this requirement; when it, or another field, is added later, that is a breaking change handled then.
- `SocketRecvFlags` is unchanged. Reporting is enabled by socket options, and no planned receive metadata needs a new receive flag.

Backends:

- The readiness selector, and therefore Native, and the Tokio adapter parse the marking from `recvmsg` control data on Linux and macOS. They handle each platform's IPv4 control message type: `IP_TOS` on Linux and `IP_RECVTOS` on macOS, both carrying one byte. They also handle `IPV6_TCLASS`, which carries an `int`. On a dual-stack socket, on Linux and macOS, they accept either form.
- io_uring's borrowed and pooled packet receives make their own nonblocking `recvmsg` through the socket's readiness monitor rather than a ring request. They pass control storage and parse the marking in the same way.
- Single-shot io_uring `RecvMsg` requests retain their control storage with the header until original completion, as they already retain address storage.
- For every multishot receiver:
  - The message template includes the control length.
  - Payloads are located after the `io_uring_recvmsg_out` header, the name and the control data.
  - Each provided buffer's metadata overhead grows by the control length, so the payload capacity configured in `RecvMsgMultishotOptions::buffer_size` is unchanged.
- Windows may report `Unsupported` initially. Windows reports ECN through `IP_ECN` and `IPV6_ECN` control messages, which need `WSARecvMsg`; the current receive path uses `WSARecvFrom`. Windows also marks only per datagram, through control messages to `WSASendMsg`, so a Windows `SendEcn` would have to attach a control message to every send. The later per-datagram design should account for this.

Release:

- These changes ship together in the next breaking release, planned as `0.6.0`. Adding the field breaks code that constructs `SocketRecvResult` with a struct literal, or destructures it without `..`. In this workspace that includes the backend and conformance crates, examples, benchmarks, integration tests and doctests. Every dependent must move from `0.5`.
- The `releases.md` entry is marked **Breaking:** and gives the migration: struct literals of `SocketRecvResult` add `ecn: None`, or the reported marking, and patterns add the field or `..`.

## Rationale

`base-http` plans a native QUIC transport and a stateless QUIC router ([QUIC and HTTP/3](../../base-http/design/http3.md), [stateless QUIC routing](../../base-http/design/quic-routing.md)). QUIC uses ECN to learn of congestion without packet loss ([RFC 9000 §13.4](https://www.rfc-editor.org/rfc/rfc9000.html#section-13.4)).

Endpoints and the router enable ECN reporting on receive. The router copies each received datagram's marking into the encapsulation it forwards, since re-sending the payload discards the original IP header; before this release it reports "unknown". Reporting `None` rather than `NotEct` when the marking is unknown lets QUIC's ECN validation and the router's encapsulation distinguish the two cases.

A per-socket marking is not enough for every sender. RFC 9000 §13.4.2 validates ECN per path, and an endpoint must stop marking on a path that fails validation, while a socket option marks every datagram the socket sends. Marking with ECT(0) therefore uses the per-socket `SendEcn` option only on a socket that carries one path, such as a client's connected socket, and turns it off when that path fails ECN validation. Servers on shared sockets, direct-server-return sockets and replies through the router do not mark until per-datagram ECN on send exists, which is a later scheduler capability.

The structures stay exhaustive: a later field, such as a datagram's destination address (`IP_PKTINFO`) or per-datagram marking on send, is a breaking change handled when it is added, rather than paying now for constructors to avoid it. Downstream repositories that only name these types in signatures, such as `base-endpoint`, `base-http` and `base-pool`, need only the dependency bump. Controlling ECN through the scheduler keeps protocol crates from reaching around its I/O capabilities.

Passing the control buffer on every receive, rather than only while reporting is enabled, removes per-socket state: options keep none today, and a record shared by a socket's readers, writers and receivers would need synchronization, would not see options set through the raw descriptor, and would need rollback and snapshot rules of its own. Reports and gets then always reflect the kernel's actual state. The cost while reporting is disabled is setting the control fields of each `msghdr`, checking an empty control area on return, and the control space in each multishot buffer, beside a system call per receive; it is measured below without a threshold. This was decided on 2026-11-20, replacing the earlier requirement that disabled receive paths pass no control storage and the per-socket record that implemented it.

The user decided on 2026-11-20:

- the `Ecn`, `ReceiveEcn` and `SendEcn` types;
- `ecn: Option<Ecn>` in `SocketRecvResult`, and the provided `ecn()` on `SocketRecvMessage`;
- `SocketRecvResult` and `SocketSendFlags` stay exhaustive, revising the earlier agreement to make them `#[non_exhaustive]`;
- release in the next breaking version, planned as `0.6.0`;
- per-datagram ECN on send later;
- the canonical marking rule above;
- no per-socket record: receives always pass the control buffer, and a get reads the kernel option, on a dual-stack socket the IPv6 option's value;
- a dual-stack set configures the IPv4 side too and fails if any call fails, without rolling back: a set that fails part-way leaves the options it already changed as the system holds them, replacing the earlier requirement to restore them;
- truncated control data reports `None`;
- Windows may return `Unsupported` initially;
- on a stream socket, `InvalidInput`, which takes precedence over `Unsupported`.

## Scope

In the interface crate:

- the `Ecn`, `ReceiveEcn` and `SendEcn` types;
- `SocketRecvResult`;
- `SocketRecvMessage` and `OwnedSocketRecvMessage`.

In the backends, the packet receive paths and the option implementations of:

- the readiness selector;
- Native;
- io_uring, including its readiness-monitor `recvmsg`, single-shot `RecvMsg` requests, `IoUringRecvMessage` and the multishot experiment;
- Tokio.

In the conformance crate, the packet ECN suite and fixtures, and its test models' `SocketRecvResult` literals.

Documentation, which is updated in the same change:

- `src/socket_recv_result.rs` and `src/socket_recv_message.rs`: the `control_truncated` documentation, which currently says the interface supplies no control buffer, and the `compile_fail` doctest's struct literal.
- `context/interface.md`: the `SocketRecvResult` description ("Ancillary bytes themselves are not exposed"), the supplied-option list (`SendEcn` is the first non-boolean option), and the message-method table (`ecn`).
- `releases.md`.

Stream sockets are outside this requirement, except that they always report `None` and reject the options.

## Validation

Add a conformance suite for packet ECN and run it against each backend over loopback, which preserves the TOS byte on Linux and macOS. Run it for IPv4, IPv6 and dual-stack sockets. The backend-controlled fixture supplies native setters for the DSCP bits, `IPV6_V6ONLY` and the kernel ECN options, since the interface has no portable options for them. The suite checks that:

- A sender using `SendEcn` with `Ect0`, and separately with `Ce`, is reported correctly by a receiver with `ReceiveEcn` enabled, through every receive form, including peeks and owned and pooled receives, and on io_uring its borrowed, pooled and single-shot owned receives.
- Reporting follows options set both before and after the reader or receiver was constructed, including multishot receivers.
- With `ReceiveEcn` disabled, every form reports `None`; with the kernel option enabled through the fixture's native setter instead, receives report the marking.
- `SendEcn` preserves DSCP bits set beforehand.
- Gets of both options return the kernel's value on IPv4, IPv6 and dual-stack sockets, the IPv6 option's value on dual-stack, including after the fixture changed the kernel options natively.
- Stream sockets reject both options with `InvalidInput`, including on a backend or platform without ECN support.
- A backend without support returns `Unsupported` from setting and getting either option on a packet socket.

Backend unit tests:

- They cover control-message parsing of both IPv4 types and `IPV6_TCLASS`, including truncated control data, intact markings alongside truncation, and unrelated control data.
- They inject a failure into the second call of a dual-stack set, using a test-only shim like the existing socket-type error shim, and check that the error is returned and that a get reports the resulting kernel state.

io_uring tests check that a multishot receiver reports the marking while reporting is enabled, including when it was enabled after the receiver was constructed, and `None` while it is disabled, with payloads and usable payload capacity unchanged.

The updated `compile_fail` doctest in `src/socket_recv_message.rs` must fail only because of its borrow error. To check this, remove the final use of the address view and confirm that the example then compiles.

Inspect each receive path to confirm that it makes no additional system call, passes control storage of the fixed size above, and keeps no per-socket reporting state, and that multishot payload capacity is unchanged.

Measure the scheduler's packet receive paths with reporting disabled, to record the control buffer's cost, using the buffer benchmark's `sockets` binary (`benchmarks/buffers/src/bin/sockets.rs`, package `base-scheduler-benchmark-buffers`) with the `udp` transport. Run it for the `tokio`, `native` and `iouring` backends in the `raw`, `pool` and `shared` modes, and for `iouring` also in `multi` and `multi-retain`. Compare against the revision before this change, built from the same benchmark source on matched hardware and build settings, and record any difference. No numerical threshold has been agreed.
