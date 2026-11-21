// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use base_scheduler::SpawnScoped;

/// An isolated scoped task service. The runner calls each case once on a thread outside the scheduler's workers, enforces a deadline, and shuts the scheduler down afterwards.
pub trait SpawnScopedFixture {
    /// A scheduler running tasks on at least two workers, so that parallel execution can be observed.
    type Scheduler: SpawnScoped + Sync;
    fn scheduler(&self) -> &Self::Scheduler;
    /// Close task admission. Subsequent spawn attempts must report `SchedulerClosed` without polling their inputs.
    fn close(&self);
}
