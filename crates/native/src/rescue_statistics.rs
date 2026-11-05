// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// Stall rescue counts returned by `NativeScheduler::rescue_statistics`, with the `diagnostics` feature.
///
/// A rescue is a pass by the monitor or an idle worker that invoked at least one waker held by a stuck worker. It is classified by what the worker was doing: a worker in idle code that is no longer sleeping is **leaving idle**; otherwise a worker that completed a task poll by the monitor's next pass, at least a millisecond after it was marked, is in a **long batch**, and one that did not is in a **stuck poll**. A stuck poll is any stall without a completed poll for that long: one poll, or a worker preempted between polls or in a long timer pass or driver turn. A poll that ends soon after the mark, or a pass delayed because the monitor was not scheduled, counts as a long batch, so a poll stuck for longer than a millisecond can go uncounted. Stuck polls in a workload without long polls are false positives; long-batch and leaving-idle rescues are normal effects of load.
///
/// Rescues made before a mark is classified are counted when it is, so they are missing from these counts while it is unclassified, and are lost if the scheduler finishes first. A rescue that races the classification can be counted with the worker's next mark instead. The total of rescued timers is counted at once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RescueStatistics {
    /// Rescues while the stuck worker was inside one poll.
    pub stuck_polls: usize,
    /// Rescues while the stuck worker was progressing through a long batch.
    pub long_batches: usize,
    /// Rescues of a worker in idle code that was no longer sleeping, between any wake, from another thread or from its own work while idle, and resuming its loop.
    pub leaving_idle: usize,
    /// Timer wakers invoked by rescues.
    pub timers: usize,
}
