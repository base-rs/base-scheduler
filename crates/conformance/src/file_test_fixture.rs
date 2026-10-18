// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use protocol_scheduler::FileScheduler;
use std::path::Path;

/// An isolated, initially absent regular-file path and independent observations for the complete file suite.
///
/// The runner owns the fixture's scheduler and temporary storage. Observations must inspect the backing store independently of the scheduler operations being tested, and panic on a mismatch. The path must stay valid until all accepted operations and cleanup retire. Use standalone assertion helpers when implementing only a subset of file capabilities.
pub trait FileTestFixture {
    type Scheduler: FileScheduler;
    fn scheduler(&self) -> &Self::Scheduler;
    fn path(&self) -> &Path;
    fn assert_absent(&self);
    fn assert_contents(&self, expected: &[u8]);
}
