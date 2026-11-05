// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// Stall rescue counts returned by `NativeScheduler::rescue_statistics`, with the `diagnostics` feature.
///
/// A rescue is a pass by the monitor or an idle worker that invoked at least one waker held by a stuck worker. It is classified by what the worker was doing: a worker in idle code that is no longer sleeping is **leaving idle**; otherwise a worker that completed a task poll by the monitor's next pass, about a millisecond after it was marked, is in a **long batch**, and one that did not is in a **stuck poll**. A poll that ends soon after the mark, or a pass delayed because the monitor was not scheduled, counts as a long batch, so stuck polls are a lower bound. Stuck polls in a workload without long polls are false positives; long-batch and leaving-idle rescues are normal effects of load.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RescueStatistics {
    /// Rescues while the stuck worker was inside one poll.
    pub stuck_polls: usize,
    /// Rescues while the stuck worker was progressing through a long batch.
    pub long_batches: usize,
    /// Rescues of a worker in idle code that was no longer sleeping: woken and waiting to run, or finishing its checks before it would park.
    pub leaving_idle: usize,
    /// Timer wakers invoked by rescues.
    pub timers: usize,
}
