# Direct allocation audit

The goal is to let ordinary I/O reuse existing buffers, cancellation state, and suitable backend capacity without fresh allocation. Backend capacity includes input, output, completion, and native descriptor storage. Direct remains a simple reference implementation, so the interface must permit reuse without requiring direct to implement an operation slab or custom completion mechanism.

## Submission and retirement

The audit covers direct's implemented file and blocking-work capabilities. Socket, message, and resolver backends are not implemented in direct; their interface allocation checks and native resolver fixture are separate evidence.

| Source | When it allocates | Reason and interface implications |
| --- | --- | --- |
| Pool `Arc<Shared>`, events, native synchronization, worker startup, and thread names | Setup or first use | Backend lifecycle and capacity costs. Native synchronization and executor parking state can initialize lazily, so reuse measurements prepare them first. |
| `VecDeque<Job>` | Queue capacity growth | Direct grows its queue as needed and retains capacity during ordinary operation. Bounded admission limits queued plus running work. Other implementations can reserve suitable queue storage in advance. |
| `Box<dyn FnOnce()>` | Each admitted job | Direct erases heterogeneous operations into a common queue. Capability methods accept concrete generic inputs and return concrete futures, so the interface does not require this box or dynamic dispatch. |
| `oneshot::channel()` | Each admitted job | Direct transfers results, panic payloads, and input owners across threads. Its completion allocation also records receiver abandonment. Backend-associated task handles and concrete operation futures permit other completion representations. |
| Cancellation and file retention | Existing-token clones and file operations do not allocate | Cloning their existing `Arc` owners retains state through retirement. The interface requires safe retention after future abandonment, but does not prescribe reference counting or a separately allocated operation record. |
| `Path::to_owned()` | Each polled file open | Direct retains a path for a potentially abandoned worker operation. The borrowed public argument cannot safely be retained by that worker after abandonment. Owned storage is necessary for this strategy; a fresh `PathBuf` is direct's choice. Reusable backend storage or a different execution strategy can satisfy the same interface. |
| `Arc<File>` | Each successfully opened resource | Direct chooses reference counting for privately retaining a file through worker access. The associated resource type leaves that representation to each backend. |
| Shutdown queue partitioning and error/panic payloads | Lifecycle or exceptional paths | Direct partitions queued jobs so user destructors run outside the queue lock, including reentrant or panicking destructors. Diagnostics and user code may allocate. These paths are outside the ordinary successful-operation measurements. |

Direct previously allocated an additional `Arc<AtomicBool>` for every general blocking job to record handle abandonment. The completion sender already reports receiver abandonment through `is_canceled()`. Using it removes one allocation per blocking submission and the custom task-handle destructor. The worker still checks abandonment and cancellation before running a general blocking closure; cancellation after that boundary does not replace its result. Accepted I/O and cleanup deliberately ignore receiver abandonment and retain their inputs until retirement.

## Evidence and limits

`blocking_pool::tests::prepared_work_allocates_only_jobs_and_completion_channels` measures repeated public `Blocking`, `FileWriteAt`, and `FileReadAt` calls with a prepared token, file, buffer, worker synchronization, and queue capacity. The submitting thread allows the two allocations retained by direct: the boxed job and completion channel. A separate measurement covers the worker loop, including execution, completion delivery, input destruction, and retirement, and requires no fresh Rust allocations for these successful operations. A no-op waker and bounded polling loop avoid including executor parking allocations in the measurements. This is a focused regression check for the simple direct implementation, not a blanket allocation requirement for conforming backends.

`tests/file_capabilities.rs` independently measures an immediate in-memory `FileRead` implementation using the same public ownership and cancellation contract, with no fresh allocation. Together with the buffer-sequence and resolver tests, this demonstrates that the interface's concrete futures, generic buffer owners, and associated result types do not require direct's job or channel allocations. The in-memory fixture does not certify an asynchronous file backend or prove throughput.

Allocation counters measure Rust allocator calls on each measured thread. They do not account for kernel memory or allocations performed privately by native libraries. User closures, new resources, capacity growth, token construction, actual cancellation propagation, diagnostic formatting, and executor setup are separate costs. Generic buffer owners, blocking captures, and outputs retain their existing unrestricted sizes; implementations may allocate or grow capacity when existing storage is unsuitable.

No further interface-imposed allocation was identified in the implemented capabilities. Real completion/readiness backends and additional capabilities should extend this audit as they are implemented. Direct keeps its boxed jobs and completion channels because they make independent worker ownership, panic delivery, and abandonment straightforward.
