# Worker and Reactor Progress Boundaries

## Requirement

Worker-driven selectors publish operation results through completion state and ordinary task wakeups. Completion collection must not poll application futures inline. When publishing a collected finite event/CQ snapshot, the owning worker finishes that snapshot before returning to application polling; another worker may consume published wakeups concurrently. A terminal driver error may stop dispatch and close/wake the affected group instead; normal snapshot publication is not a requirement to continue using a failed driver.

Before entering a blocking wait, the worker/driver combination must preserve racing runnable publications, accepted commands, timer changes and shutdown notifications through rechecking or a retained interrupt/permit. A driver which has published work that can make tasks runnable must return to the executor instead of waiting for its own thread to interrupt it. An io_uring output caller owed an issuance acknowledgment must be allowed to resume without first waiting for that output's completion; staging alone is not issuance.

Progress optimizations must preserve the existing cancellation, buffer ownership, original-operation retirement and shutdown contracts in [the interface design](../context/design.md). Ready borrowed transfers complete their payload access within the poll; reactor batching cannot retain borrowed payload pointers. Registration reclamation must account for collected events and all kernel-visible lifetimes.

## Rationale

The HTTP performance investigation found both avoidable per-operation work and results delayed across worker/reactor boundaries. A cohesive model guides subsequent research and implementation. Batching necessary work must not lose wakeups, require an interrupt from the executing thread, conflate issuance with completion, or shorten resource ownership.

## Scope

Native workers composed with worker-driven readiness or io_uring selectors, including idle transitions, completion publication, output issuance, cancellation, abandonment and shutdown. Independently driven selectors retain their operation/ownership contracts but do not adopt Native's ready-queue policy.

This document does not require a fixed task budget, preemption, a maximum batch duration, a numerical latency guarantee, or one universal submission policy. The current two-queue policy and candidate policies are explained in [the execution guide](../context/execution.md).

## Validation

Inspect worker queue publication, idle fencing/rechecking, driver wait decisions, event/CQ dispatch and reclamation, and issuance receipt accounting. Exercise local and remote wakes racing idle preparation, callback reentrancy, busy command queues, immediate completions, abandoned operations, cancellation before original completion, pending output receipts, and graceful driver shutdown. Apply the existing urgent-data requirement to affected socket paths.

For performance changes, identify the cost or stage delay being removed, retain the baseline/source configuration, and compare equivalent operation semantics, task topology, CPU placement and build settings. Measure fixed-rate latency at the same offered rate separately from unrestricted throughput and process CPU. Report noise and failed or inconclusive experiments. No numerical regression threshold has been agreed; measurements justify a policy choice rather than creating an implicit service-level guarantee.
