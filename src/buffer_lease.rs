// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{FrozenBuffer, IoBuf, IoBufMut, heap_buffer_pool::Pool};
use std::{
    ops::Range,
    sync::{Arc, atomic::Ordering},
};

/// An exclusive writable pool lease. Its initialized length can be shortened after a read, without copying or borrowing the receiver. Drop returns storage to the pool; an I/O backend must retain this owner until its access retires.
pub struct BufferLease {
    pub(crate) pool: Option<Arc<Pool>>,
    pub(crate) index: usize,
    length: usize,
}

impl BufferLease {
    pub(crate) fn new(pool: Arc<Pool>, index: usize, length: usize) -> Self {
        Self {
            pool: Some(pool),
            index,
            length,
        }
    }

    /// Limit the visible initialized prefix. Panics if the requested length exceeds the current prefix.
    pub fn truncate(&mut self, length: usize) {
        assert!(length <= self.length, "buffer length out of bounds");
        self.length = length;
    }

    /// Transfer this lease into immutable, independently retainable storage. No allocation or payload copy is required.
    pub fn freeze(mut self) -> FrozenBuffer {
        let pool = self.pool.take().unwrap();
        pool.slots[self.index].readers.store(1, Ordering::Relaxed);
        FrozenBuffer::new(
            pool,
            self.index,
            Range {
                start: 0,
                end: self.length,
            },
        )
    }
}

impl AsRef<[u8]> for BufferLease {
    fn as_ref(&self) -> &[u8] {
        // SAFETY: This lease exclusively owns its slot; the box and initialized prefix remain stable until release.
        let pool = self.pool.as_ref().unwrap();
        unsafe { std::slice::from_raw_parts(pool.pointer(self.index), self.length) }
    }
}
impl AsMut<[u8]> for BufferLease {
    fn as_mut(&mut self) -> &mut [u8] {
        // SAFETY: The unique lease and exclusive borrow prevent other accesses to this slot. Frozen views cannot coexist with a mutable lease.
        let pool = self.pool.as_ref().unwrap();
        unsafe { std::slice::from_raw_parts_mut(pool.pointer(self.index), self.length) }
    }
}
impl Drop for BufferLease {
    fn drop(&mut self) {
        if let Some(pool) = self.pool.take() {
            pool.release(self.index);
        }
    }
}

// SAFETY: Pool storage is initialized, fixed in size, and stable as its exclusive lease moves. Only the owned lease permits mutation; operations retain it through retirement.
unsafe impl IoBuf for BufferLease {}
unsafe impl IoBufMut for BufferLease {}

impl std::fmt::Debug for BufferLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BufferLease")
            .field("length", &self.length)
            .finish_non_exhaustive()
    }
}

impl crate::ReceiveBuffer for BufferLease {
    type Frozen = FrozenBuffer;
    fn truncate(&mut self, length: usize) {
        BufferLease::truncate(self, length);
    }
    fn freeze(self) -> FrozenBuffer {
        BufferLease::freeze(self)
    }
}
