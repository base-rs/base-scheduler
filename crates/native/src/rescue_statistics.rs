// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// Stall rescue counts returned by `NativeScheduler::rescue_statistics`, with the `diagnostics` feature.
///
/// A rescue is a pass by the monitor or an idle worker that invoked at least one waker held by a stuck worker. It is classified by whether the stuck worker completed any task poll since the monitor's previous pass: none means a **stuck poll**, some means a **long batch**. That pass can be long before the stall began, so the classification is approximate. Stuck polls in a workload without long polls are false positives; long-batch rescues are a normal effect of load.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RescueStatistics {
    /// Rescues while the stuck worker was inside one poll.
    pub stuck_polls: usize,
    /// Rescues while the stuck worker was progressing through a long batch.
    pub long_batches: usize,
    /// Timer wakers invoked by rescues.
    pub timers: usize,
}
