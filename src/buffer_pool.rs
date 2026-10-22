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
        Arc, Mutex,
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
    free: Mutex<Vec<usize>>,
    returned: Event,
    size: usize,
}

// SAFETY: A slot is either on the free list, held exclusively by one mutable lease, or held by immutable views counted in readers. Only the exclusive lease can mutate its bytes. Final release synchronizes through the free-list mutex before another lease can access that slot. Slots and byte allocations never move or resize.
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
        let free = Mutex::new((0..buffers).rev().collect());
        // Initialize native mutex state during pool setup rather than the first checkout.
        drop(free.lock().unwrap());
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
        self.0
            .free
            .lock()
            .unwrap()
            .pop()
            .map(|index| BufferLease::new(Arc::clone(&self.0), index, self.0.size))
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
        self.free.lock().unwrap().push(index);
        self.returned.notify(1);
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
