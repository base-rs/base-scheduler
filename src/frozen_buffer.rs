// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{BufferSlice, IoBuf, buffer_pool::Pool};
use std::{
    ops::{Range, RangeBounds},
    sync::Arc,
};

/// An immutable pool-backed chunk. Cloning or retaining a range shares its existing allocation; the final view returns the slot to the pool. Small views can retain a large slot, so parsers may choose to copy unusually long-lived fragments.
pub struct FrozenBuffer {
    pool: Arc<Pool>,
    index: usize,
    range: Range<usize>,
}
impl FrozenBuffer {
    pub(crate) fn new(pool: Arc<Pool>, index: usize, range: Range<usize>) -> Self {
        Self { pool, index, range }
    }

    /// Retain an independently owned range without copying payload bytes or allocating a reference-count wrapper.
    pub fn view(&self, range: impl RangeBounds<usize>) -> BufferSlice<Self> {
        self.clone().slice(range)
    }
}
impl Clone for FrozenBuffer {
    fn clone(&self) -> Self {
        // Clone the pool owner first: Arc's overflow guard bounds the number of live slot readers too.
        let pool = Arc::clone(&self.pool);
        pool.retain(self.index);
        Self::new(pool, self.index, self.range.clone())
    }
}
impl AsRef<[u8]> for FrozenBuffer {
    fn as_ref(&self) -> &[u8] {
        // SAFETY: Every view retains the slot's reader count and pool ownership. No mutable lease can exist until the final view releases it.
        unsafe { &(&*self.pool.slots[self.index].bytes.get())[self.range.clone()] }
    }
}
impl Drop for FrozenBuffer {
    fn drop(&mut self) {
        self.pool.release_reader(self.index);
    }
}
// SAFETY: Shared views expose only immutable initialized bytes at a stable allocation and range, retained through the last view's release.
unsafe impl IoBuf for FrozenBuffer {}

impl std::fmt::Debug for FrozenBuffer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FrozenBuffer")
            .field("length", &self.range.len())
            .finish_non_exhaustive()
    }
}
