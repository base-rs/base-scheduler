// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{BufferLease, Cancellation, Cancelled};
use event_listener::Event;
use std::{
    cell::UnsafeCell,
    future::{Future, poll_fn},
    io,
    pin::pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::Poll,
};

/// A bounded, shareable collection of stable initialized allocations. Construction reserves all payload storage; acquiring, freezing, slicing, and returning leases allocate no payload or reference-count wrappers.
///
/// Pools may be shared across connections and file pipelines. Exhaustion waits with cancellation instead of growing the pool. Retained views count against capacity. This portable pool does not register memory with a kernel; backend-specific pools may provide the same ownership model independently.
#[derive(Clone)]
pub struct BufferPool(pub(crate) Arc<Pool>);

pub(crate) struct Slot {
    pub(crate) bytes: UnsafeCell<Box<[u8]>>,
    pub(crate) readers: AtomicUsize,
}

pub(crate) struct Pool {
    pub(crate) slots: Box<[Slot]>,
    // One bit per stable slot; the final word never advertises unused indices.
    free: Box<[AtomicUsize]>,
    returned: Event,
    size: usize,
}

// SAFETY: A set availability bit, one mutable lease, or immutable views counted in readers exclusively owns each slot. Only the mutable lease can mutate bytes. Clearing an availability bit acquires the final owner's release before accessing that slot. Slots and byte allocations never move or resize.
unsafe impl Sync for Pool {}

impl BufferPool {
    pub fn new(buffers: usize, buffer_size: usize) -> io::Result<Self> {
        if buffers == 0 || buffer_size == 0 || buffers.checked_mul(buffer_size).is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "nonzero buffer count and size with a representable total are required",
            ));
        }
        let returned = Event::new();
        returned.notify(0);
        let width = usize::BITS as usize;
        let free = (0..buffers.div_ceil(width))
            .map(|word| {
                let bits = (buffers - word * width).min(width);
                AtomicUsize::new(usize::MAX >> (width - bits))
            })
            .collect();
        Ok(Self(Arc::new(Pool {
            slots: (0..buffers)
                .map(|_| Slot {
                    bytes: UnsafeCell::new(vec![0; buffer_size].into_boxed_slice()),
                    readers: AtomicUsize::new(0),
                })
                .collect(),
            free,
            returned,
            size: buffer_size,
        })))
    }

    pub fn capacity(&self) -> usize {
        self.0.slots.len()
    }
    pub fn buffer_size(&self) -> usize {
        self.0.size
    }

    /// Acquire exclusive writable storage immediately, or return `None` without allocating.
    pub fn try_acquire(&self) -> Option<BufferLease> {
        for (word_index, word) in self.0.free.iter().enumerate() {
            let mut available = word.load(Ordering::Relaxed);
            while available != 0 {
                match word.compare_exchange_weak(
                    available,
                    available & (available - 1),
                    Ordering::Acquire,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        let index =
                            word_index * usize::BITS as usize + available.trailing_zeros() as usize;
                        return Some(BufferLease::new(Arc::clone(&self.0), index, self.0.size));
                    }
                    Err(current) => available = current,
                }
            }
        }
        None
    }

    /// Wait for one lease. Pre-cancellation leaves capacity unchanged; a successfully acquired lease wins a later cancellation race.
    pub async fn acquire(&self, cancellation: &Cancellation) -> Result<BufferLease, Cancelled> {
        cancellation.check()?;
        if let Some(buffer) = self.try_acquire() {
            return Ok(buffer);
        }
        let mut cancelled = pin!(cancellation.cancelled());
        loop {
            event_listener::listener!(self.0.returned => listener);
            if let Some(buffer) = self.try_acquire() {
                return Ok(buffer);
            }
            let mut listener = pin!(listener);
            poll_fn(|context| {
                if cancelled.as_mut().poll(context).is_ready() {
                    return Poll::Ready(Err(Cancelled));
                }
                listener.as_mut().poll(context).map(|_| Ok(()))
            })
            .await?;
        }
    }
}

impl Pool {
    pub(crate) fn release(&self, index: usize) {
        let width = usize::BITS as usize;
        // Use an RMW to preserve release sequences for other slots in the same word.
        self.free[index / width].fetch_or(1 << (index % width), Ordering::Release);
        self.returned.notify_additional(1);
    }

    pub(crate) fn retain(&self, index: usize) {
        // Every reader first acquires an Arc owner; Arc's overflow guard prevents this count from wrapping.
        self.slots[index].readers.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn release_reader(&self, index: usize) {
        if self.slots[index].readers.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.release(index);
        }
    }
}
