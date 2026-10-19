# Core Scheduler Selector

I/O capabilities independent of an executor. `ReadinessSelector` supplies socket operations through `async-io`, file and resolver operations through bounded native workers, and platform-supported signals. It can be polled from any executor while its services remain alive.

`Selector` describes admission and asynchronous retirement. Individual operations use the existing `core-scheduler` capability traits. Concrete selectors document how their event driver is run; the lifecycle interface does not create threads or require a universal set of operations.

Call `shutdown().await` after coordinating application cancellation and cleanup. Dropping a selector closes admission without synchronously waiting; backend workers retain accepted I/O until access ends.

`SelectorDriver` and `SelectorWorker` support executor-owned event loops. A movable driver enters one worker, consuming itself into a possibly thread-bound owner. That owner provides interruptible waits and bounded completion batches. Completion processing wakes tasks; the executor polls its ready queue only after the driver returns. Selectors with independent progress, such as `ReadinessSelector`, do not need to implement this optional driving contract.
