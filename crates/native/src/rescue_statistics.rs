// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// Stall rescue counts returned by `NativeScheduler::rescue_statistics`, with the `diagnostics` feature.
///
/// A rescue is a pass by the monitor or an idle worker that invoked at least one waker held by a stuck worker. It is classified by what the worker was doing: a worker woken from idle that had not yet resumed is a **woken worker**; otherwise a worker that completed a task poll within about a millisecond of being marked is in a **long batch**, and one that did not is in a **stuck poll**. Stuck polls in a workload without long polls are false positives; long-batch and woken-worker rescues are normal effects of load.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RescueStatistics {
    /// Rescues while the stuck worker was inside one poll.
    pub stuck_polls: usize,
    /// Rescues while the stuck worker was progressing through a long batch.
    pub long_batches: usize,
    /// Rescues of a worker woken from idle that had not yet resumed running.
    pub woken: usize,
    /// Timer wakers invoked by rescues.
    pub timers: usize,
}
