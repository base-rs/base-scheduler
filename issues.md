# Allocation efficiency

Ordinary I/O should require no fresh allocation when buffers, cancellation state, and backend capacity already exist. Backend capacity includes storage suitable for the operation's inputs, result, and completion state, not just an available worker or queue entry.

The interface must permit this goal. It is not a zero-allocation requirement for every backend. Direct is a reference implementation whose priorities are clarity, simplicity, and passing the behavioral conformance suite.

Track completed work and proposed changes below. Preserve the ownership, cancellation, and retirement guarantees in [the scheduler design](context/design.md).

## 1. Remove per-wait cancellation allocations

Status: implemented, with allocation regression tests.

`Cancellation::cancelled()` previously used `Event::listen()`, which heap-allocates its listener. It now retains listeners inline and initializes shared event state during token construction rather than on the first wait.

Optimize the ordinary path where work proceeds without cancellation: cheap cancellation checks during progress, no listener registration when an operation can complete immediately, and allocation-free registration when it must wait. Preserve the cancellation contract when taking an immediate-completion path. Returning early from a wait on an already-cancelled token is a secondary optimization.

- Use `event_listener::listener!` to retain pinned listener storage inside the waiting future.
- Inline storage removes the separate listener allocation. It lives inside the future, which may itself be stack-allocated or executor-allocated; pinning provides address stability without imposing thread affinity on a `Send` future.
- Initialize shared event state during token construction so the first wait does not introduce a deferred setup allocation.
- Preserve registration before the final cancellation check to avoid missed notifications, and unregister safely when the future is dropped.
- Apply the same approach to the direct backend's capacity and shutdown waiters. Avoid registering a waiter when capacity is immediately available, with a recheck after registration when waiting is necessary.
- Reuse the supplied cancellation state rather than creating a child token for each operation. Keep token construction and child registration as lifecycle operations that may allocate. More work on actual cancellation is acceptable; cancellation propagation currently uses temporary vectors, and making `cancel()` itself allocation-free is a separate follow-up.

Acceptance: checking, cloning, and waiting on an existing initialized token require no fresh allocation. Verify that immediate-completion paths avoid unnecessary cancellation listener registration while preserving the cancellation contract. Tests must retain coverage of cancellation races, dropped waits, and `Send` futures.

## 2. Remove prescribed `Vec` representations from message I/O and resolution

Status: implemented in the interface, with allocation and ownership regression tests. Concrete socket and resolver backends remain separate work. Point (3) remains follow-up work.

### Message buffers

`SocketMessage` and `SocketRecvMsg` previously prescribed `Vec<B>`. They now accept `IoBufSequence` and `IoBufSequenceMut` owners, including fixed-size collections of buffer owners.

- Introduce an owned buffer-sequence abstraction supporting arrays, vectors, and pool-backed collections.
- Take ownership of the collection and return the same owner on success, error, or cancellation.
- Preserve stable byte storage and resource retention until backend access ends. An array of buffer owners may move; the buffers' byte storage must remain stable.
- Allow backends to reuse native scatter/gather descriptor storage rather than constructing a fresh allocated descriptor list for every operation.

Acceptance: fixed-size and reusable dynamic message collections work without fresh collection allocations. Verify owner recovery, partial transfers, cancellation, and backend retention after future abandonment.

### Address resolution

`AddressResolution` previously returned `Vec<AddressInfo>`, requiring a native resolver to materialize a separate Rust collection of normalized records. Its associated `Addresses` owner now supplies `AddressInfoCollection` and borrowed `AddressInfoView` accessors. Follow the same pattern as `FileStat::Metadata` and the native `StatxMetadata` wrapper: retain the implementation's representation and expose semantic accessors.

- Give `AddressResolution` an associated owned result type, with borrowed iteration over entry views whose lifetimes are tied to that owner.
- Let native implementations own the original `addrinfo` list, traverse `ai_next` directly, and release it exactly once through the matching platform deallocator on drop. Do not require an extra vector, per-entry allocation, or eager field conversion.
- Provide portable accessors for address, socket type, and protocol. Decode a small `SocketAddr` value only when requested; this conversion needs no heap allocation. Keep native-specific access available through concrete wrappers.
- Permit non-native resolvers to supply arrays, vectors, or cached storage through the same semantic interface. Do not require native layouts, cloning the result owner, or reference counting each entry.
- Preserve candidate ordering, duplicates, IPv6 scope identifiers, native type/protocol values, validation, and cancellation/abandonment behavior. Validation of a native list need not copy it.

Acceptance: native results can be wrapped and iterated without additional collection allocations or eager normalization. Verify borrowed-view lifetimes, exactly-once cleanup including early iteration termination and abandoned results, and composition with a non-native resolver representation. Allocations performed by the native resolver itself remain part of its result creation.

## 3. Use direct to identify allocations forced by the interface

Status: audited, with a straightforward blocking-submission improvement and allocation regression tests. See the [direct allocation audit](context/allocation-audit.md) for costs, ownership constraints, and measurement limits. No further interface-imposed allocation was identified in the implemented capabilities.

The direct backend currently allocates boxed jobs and completion channels per submission. These can be reasonable implementation choices for a simple reference backend. The important question is whether the interface forces an allocation that an implementation should be able to avoid.

General blocking submissions now reuse the completion channel's receiver-abandonment state instead of allocating a separate shared flag. Prepared blocking and positioned-file operations retain only direct's job and channel allocations on the submitting thread, with no fresh Rust allocation in the measured worker loop. An immediate in-memory file implementation separately demonstrates allocation-free composition through the public interface. Accepted I/O and cleanup continue through retirement after their waiters are abandoned.

- Trace allocation costs exposed by direct back to their source: interface requirements, shared support code, or backend implementation choices.
- Fix interface requirements that unnecessarily prevent an implementation from using inline or reusable storage. Demonstrate the alternative with a focused implementation or test where useful; direct does not have to adopt that storage strategy.
- Take straightforward performance wins in direct when they preserve clarity and simplicity, such as inline waiters or avoiding unnecessary waiter registration when capacity is available.
- Retain boxed jobs and completion channels where they make ownership and lifecycle behavior easier to understand. Reusable operation slabs, custom completion machinery, and size-class storage are not requirements for direct.
- Preserve bounded admission, separate cleanup capacity, queued cancellation, panic propagation, and retirement after future or scheduler abandonment.
- Do not impose artificial size limits on generic buffer owners or `Blocking` closures to satisfy an allocation target.

Acceptance: identify and address avoidable interface-imposed allocations. Direct remains a clear reference implementation that passes the behavioral conformance suite; it has no blanket zero-allocation acceptance criterion.

## Allocation regression tests

Use isolated allocation-counting tests for paths intended to require no fresh allocation, preparing buffers, tokens, backend capacity, and executor infrastructure before measurement. Include worker threads when measuring work dispatched to them. Cover cancellation waits and reusable message collections, and use focused implementations to demonstrate that the interface permits allocation-free ordinary I/O. Measure setup and capacity growth separately from reuse.

Keep allocation checks separate from behavioral conformance. Measurements of direct should help distinguish interface restrictions from implementation costs, without requiring every direct operation or conforming backend to allocate nothing.
