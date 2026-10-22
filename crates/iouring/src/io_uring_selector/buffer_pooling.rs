// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

impl BufferPooling for IoUringSelector {
    type Pool = HeapBufferPool;
    fn buffer_pool(&self, options: BufferPoolOptions) -> std::io::Result<Self::Pool> {
        self.check_open()?;
        HeapBufferPool::with_options(options)
    }
}
