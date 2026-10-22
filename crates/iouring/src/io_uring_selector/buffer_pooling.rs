// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

impl BufferPooling for IoUringSelector {
    type Pool = crate::IoUringBufferPool;
    fn buffer_pool(&self, options: BufferPoolOptions) -> std::io::Result<Self::Pool> {
        self.check_open()?;
        crate::IoUringBufferPool::new(&self.group, options)
    }
}

impl IoUringSelector {
    /// Experimental worker-owned receive-ahead storage. Generic applications use `BufferPooling::buffer_pool`; this concrete backend control exists for comparing provided-buffer strategies.
    ///
    /// Reserve `options.buffers` slots on every I/O worker, with `capacity()` reporting the total. Acquisition waits for its selected shard even when another shard has free slots. Completed leases can move between workers and always return to their original storage. This is a different capacity contract from the portable factory.
    ///
    /// Construction allocates every worker's storage and queues registration for its next driver turn, including startup. It does not wait for unstarted workers. Registration errors are reported when preparing receivers. Published slots belong to accepted receives, including idle ones; use a separate ordinary pool for independently required scratch storage or file pipelines.
    pub fn buffer_pool_per_worker(
        &self,
        options: BufferPoolOptions,
    ) -> std::io::Result<crate::IoUringBufferPool> {
        self.check_open()?;
        crate::IoUringBufferPool::per_worker(&self.group, &self.controllers, options)
    }
}
