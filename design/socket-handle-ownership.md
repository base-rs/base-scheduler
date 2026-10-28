# Socket Handle Ownership

## Requirement

All stream and packet reader/writer factories return handles whose lifetimes may depend on the scheduler and declared type parameters, but not the temporary socket reference passed to construction. This includes supplied-pool readers and custom-buffer writers. Generic callers can move a socket and its directional handles into one owner without self references, pinning, unsafe lifetime extension or dynamic dispatch.

Backends retain socket access independently of the temporary reference. The handles remain bound to their constructing scheduler. Applications retire and drop directional handles before exclusively closing the socket; moving ownership does not establish retirement.

The workspace minimum Rust version is 1.87, which supports precise capture bounds in trait return types. Backend implementations explicitly exclude the socket-reference lifetime in their return types. Implementations written against the previous implicitly capturing signatures require a corresponding signature update.

## Rationale

The pooled HTTP client needs to retain scheduler-created transport capabilities alongside connection ownership. The previous trait return types implicitly captured every input lifetime, despite concrete implementations retaining their socket independently. Correcting that contract keeps a temporary borrow from dictating protocol-driver architecture.

## Scope

The seven stream/packet factory variants, concrete backend and Native forwarding implementations, and the workspace minimum Rust version. Data-transfer, cancellation, urgent-data and retirement behavior remain unchanged.

## Validation

Compile generic factory consumers that return a moved socket with every stream and packet reader/writer variant. Exercise construction and explicit retirement on a concrete backend. Validate forwarding through Native and an implementation whose selector borrows local state. Run the workspace tests on Rust 1.87 and current supported targets, including Linux io_uring.
