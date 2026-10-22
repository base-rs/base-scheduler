// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Worker-owned storage and publication. Pool construction allocates every slab and ring; only the owning driver registers and publishes them. Leases return directly to their original slab from any thread.

use super::*;
use crate::io_uring_buffer_pool::RECEIVE_METADATA;
use std::{
    alloc::{Layout, alloc_zeroed, dealloc},
    pin::Pin,
    ptr::NonNull,
    sync::{Mutex, atomic::AtomicU16},
    task::{Context, Waker},
};

/// A reusable capacity waiter. Its async listener storage is allocated once per worker, rather than once per exhausted receive batch.
struct Acquisition {
    future: Pin<Box<dyn Future<Output = ()> + Send>>,
    output: Arc<Mutex<Option<BufferLease>>>,
}
impl Acquisition {
    fn new(heap: HeapBufferPool) -> Self {
        let output = Arc::new(Mutex::new(None));
        let acquired = Arc::clone(&output);
        let future = Box::pin(async move {
            loop {
                let buffer = heap
                    .acquire(&Cancellation::never())
                    .await
                    .expect("the internal capacity wait is never cancelled");
                *acquired.lock().unwrap() = Some(buffer);
                // Hand one lease back to the driver before starting the next acquisition. The driver polls again only when it needs more storage.
                let mut yielded = false;
                poll_fn(|_| {
                    if std::mem::replace(&mut yielded, true) {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                })
                .await;
            }
        });
        Self { future, output }
    }
    fn next(&mut self, waker: &Waker) -> Option<BufferLease> {
        if self
            .future
            .as_mut()
            .poll(&mut Context::from_waker(waker))
            .is_ready()
        {
            unreachable!("the reusable acquisition loop yields leases without terminating");
        }
        self.output.lock().unwrap().take()
    }
}

struct Ring {
    group: Arc<multishot::pool::Group>,
    pointer: NonNull<types::BufRingEntry>,
    layout: Layout,
    entries: u16,
    registered: bool,
    failure: Option<io::Error>,
}
// SAFETY: The allocation is stable. Registered access is confined to its owning driver; other threads only inspect worker state under its mutex.
unsafe impl Send for Ring {}

impl Ring {
    fn new(group: Arc<multishot::pool::Group>, buffers: usize) -> io::Result<Self> {
        let entries = buffers.next_power_of_two() as u16;
        // SAFETY: Linux supports _SC_PAGESIZE and returns a positive power of two.
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;
        let layout = Layout::from_size_align(
            entries as usize * std::mem::size_of::<types::BufRingEntry>(),
            page,
        )
        .unwrap();
        let pointer = syscall("allocate-buffer-ring", || {
            // SAFETY: The nonzero valid layout is retained until deregistration.
            NonNull::new(unsafe { alloc_zeroed(layout) }.cast())
                .ok_or(io::ErrorKind::OutOfMemory.into())
        })?;
        Ok(Self {
            group,
            pointer,
            layout,
            entries,
            registered: false,
            failure: None,
        })
    }
    fn register(&mut self, submitter: &io_uring::Submitter<'_>) -> io::Result<()> {
        if !self.registered && self.failure.is_none() {
            match syscall("register-buffer-ring", || {
                // SAFETY: WorkerState retains this page-aligned ring and every published lease until all original receives retire and registration is removed.
                unsafe {
                    submitter.register_buf_ring_with_flags(
                        self.pointer.as_ptr() as u64,
                        self.entries,
                        self.group.identifier,
                        0,
                    )
                }
            }) {
                Ok(()) => self.registered = true,
                Err(error) => self.failure = Some(error),
            }
        }
        match &self.failure {
            Some(error) => Err(match error.raw_os_error() {
                Some(code) => io::Error::from_raw_os_error(code),
                None => io::Error::new(error.kind(), error.to_string()),
            }),
            None => Ok(()),
        }
    }
    fn unregister(&mut self, submitter: &io_uring::Submitter<'_>) -> io::Result<()> {
        if self.registered {
            syscall("unregister-buffer-ring", || {
                submitter.unregister_buf_ring(self.group.identifier)
            })?;
            self.registered = false;
            // SAFETY: No original receive remains, and successful deregistration ends kernel access to the ring. Reset before its next registration.
            unsafe {
                std::ptr::write_bytes(self.pointer.as_ptr().cast::<u8>(), 0, self.layout.size())
            };
        }
        Ok(())
    }
    fn publish(&self, identifier: usize, buffer: &mut BufferLease, offset: usize) {
        // SAFETY: The first entry's reserved bytes alias the ABI's shared atomic tail.
        let tail = unsafe {
            AtomicU16::from_ptr(types::BufRingEntry::tail(self.pointer.as_ptr()).cast_mut())
        };
        let position = tail.load(Ordering::Relaxed);
        // SAFETY: Zero is a valid initial entry; setters fill all fields consumed by the kernel.
        let mut entry: types::BufRingEntry = unsafe { std::mem::zeroed() };
        entry.set_addr(buffer.as_mut()[offset..].as_mut_ptr() as u64);
        entry.set_len((buffer.as_ref().len() - offset) as u32);
        entry.set_bid(identifier as u16);
        // SAFETY: The caller exclusively owns this unpublished lease. Its slot is free and the ring has capacity. Copying 14 bytes preserves slot zero's atomic tail.
        unsafe {
            std::ptr::copy_nonoverlapping(
                (&entry as *const types::BufRingEntry).cast::<u8>(),
                self.pointer
                    .as_ptr()
                    .add((position & (self.entries - 1)) as usize)
                    .cast(),
                14,
            )
        };
        tail.store(position.wrapping_add(1), Ordering::Release);
    }
}
impl Drop for Ring {
    fn drop(&mut self) {
        if !self.registered {
            // SAFETY: No kernel reference survives successful deregistration or a never-registered allocation.
            unsafe { dealloc(self.pointer.as_ptr().cast(), self.layout) };
        }
        // An unexpected driver failure deliberately leaks registered metadata, matching retained in-flight payloads.
    }
}

struct WorkerState {
    ring: Ring,
    buffers: Box<[Option<BufferLease>]>,
    vacant: Vec<usize>,
    users: Vec<u64>,
    active: usize,
    available: usize,
    acquisition: Option<Acquisition>,
}
impl WorkerState {
    fn reset(&mut self, submitter: &io_uring::Submitter<'_>) -> io::Result<()> {
        assert_eq!(
            self.active, 0,
            "original receives retire before unpublishing storage"
        );
        self.ring.unregister(submitter)?;
        for buffer in &mut self.buffers {
            drop(buffer.take());
        }
        self.vacant.clear();
        self.vacant.extend((0..self.buffers.len()).rev());
        self.available = 0;
        self.acquisition = None;
        Ok(())
    }
}
impl Drop for WorkerState {
    fn drop(&mut self) {
        if self.ring.registered {
            // A fatal driver error may leave kernel access outstanding. Retain every still-published lease as well as the ring metadata.
            for buffer in self.buffers.iter_mut().filter_map(Option::take) {
                std::mem::forget(buffer);
            }
        }
    }
}

pub(crate) struct WorkerBuffers {
    pub(crate) controller: Weak<Controller>,
    pub(crate) heap: HeapBufferPool,
    options: BufferPoolOptions,
    identifier: u16,
    released: AtomicBool,
    state: Mutex<WorkerState>,
}
impl WorkerBuffers {
    pub(crate) fn new(
        heap: HeapBufferPool,
        options: BufferPoolOptions,
        group: Arc<multishot::pool::Group>,
        controller: &Arc<Controller>,
    ) -> io::Result<Arc<Self>> {
        let identifier = group.identifier;
        let ring = Ring::new(group, options.buffers)?;
        Ok(Arc::new(Self {
            controller: Arc::downgrade(controller),
            heap,
            options,
            identifier,
            released: AtomicBool::new(false),
            state: Mutex::new(WorkerState {
                ring,
                buffers: (0..options.buffers).map(|_| None).collect(),
                vacant: (0..options.buffers).rev().collect(),
                users: Vec::new(),
                active: 0,
                available: 0,
                acquisition: None,
            }),
        }))
    }
    pub(crate) fn release(&self) {
        self.released.store(true, Ordering::Release);
        if let Some(controller) = self.controller.upgrade() {
            Waker::from(controller).wake();
        }
    }
    pub(super) fn attach(&self, identifier: u64) {
        self.state.lock().unwrap().users.push(identifier);
    }
    pub(super) fn abandon(&self, identifier: u64) {
        self.state
            .lock()
            .unwrap()
            .users
            .retain(|user| *user != identifier);
    }
    pub(crate) fn group(&self) -> u16 {
        self.identifier
    }
    pub(super) fn has_buffers(&self) -> bool {
        self.state.lock().unwrap().available != 0
    }
    pub(super) fn submitted(&self) {
        self.state.lock().unwrap().active += 1;
    }
    pub(super) fn completed(&self) {
        self.state.lock().unwrap().active -= 1;
    }
    pub(super) fn take(&self, identifier: u16) -> BufferLease {
        let mut state = self.state.lock().unwrap();
        let index = identifier as usize;
        let buffer = state.buffers[index]
            .take()
            .expect("a selected ID owns its published lease");
        state.vacant.push(index);
        state.available -= 1;
        buffer
    }
    pub(super) fn empty(&self) -> bool {
        self.released.load(Ordering::Acquire) && self.state.lock().unwrap().users.is_empty()
    }
    pub(super) fn publish_offset(&self) -> usize {
        let alignment = self.options.alignment;
        ((RECEIVE_METADATA + alignment - 1) & !(alignment - 1)) - RECEIVE_METADATA
    }
    pub(super) fn detach(
        &self,
        identifier: u64,
        submitter: &io_uring::Submitter<'_>,
    ) -> io::Result<()> {
        let mut state = self.state.lock().unwrap();
        state.users.retain(|user| *user != identifier);
        if state.users.is_empty() {
            state.reset(submitter)?;
            if !self.released.load(Ordering::Acquire) {
                // Restore an initialized empty registration. Ordinary acquisitions can now use all unconsumed capacity without competing with a dormant receive.
                state.ring.register(submitter)?;
            }
        }
        Ok(())
    }
    pub(super) fn shutdown(&self, submitter: &io_uring::Submitter<'_>) -> io::Result<()> {
        let mut state = self.state.lock().unwrap();
        if state.active == 0 {
            state.reset(submitter)?;
        }
        Ok(())
    }
    pub(super) fn prepare(&self, submitter: &io_uring::Submitter<'_>) -> io::Result<()> {
        let mut state = self.state.lock().unwrap();
        state.ring.register(submitter)?;
        self.refill(&mut state);
        Ok(())
    }
    /// Initialize even before the first receiver, then refill from this worker's own slab. No other worker's registrations or free capacity participate.
    pub(super) fn service(&self, submitter: &io_uring::Submitter<'_>) -> io::Result<bool> {
        let mut state = self.state.lock().unwrap();
        if self.empty_without_lock(&state) {
            state.reset(submitter)?;
            return Ok(false);
        }
        // Registration failures belong to this pool, not the entire scheduler. Preserve the original error for every receiver's prepare call.
        if state.ring.register(submitter).is_err() {
            return Ok(false);
        }
        if state.users.is_empty() {
            if state.available != 0 || state.acquisition.is_some() {
                state.reset(submitter)?;
                state.ring.register(submitter)?;
            }
            return Ok(false);
        }
        self.refill(&mut state);
        // Availability remains observable even when prepare() performed the refill or another receive left unused entries. Only the reactor consumes dormant requests.
        Ok(state.available != 0)
    }
    fn refill(&self, state: &mut WorkerState) {
        let offset = self.publish_offset();
        while let Some(&index) = state.vacant.last() {
            let buffer = self.heap.try_acquire().or_else(|| {
                if state.available != 0 && state.acquisition.is_none() {
                    return None;
                }
                let controller = self
                    .controller
                    .upgrade()
                    .expect("a driven pool retains its controller");
                state
                    .acquisition
                    .get_or_insert_with(|| Acquisition::new(self.heap.clone()))
                    .next(&Waker::from(controller))
            });
            let Some(mut buffer) = buffer else { break };
            state.ring.publish(index, &mut buffer, offset);
            state.buffers[index] = Some(buffer);
            state.vacant.pop();
            state.available += 1;
        }
    }
    fn empty_without_lock(&self, state: &WorkerState) -> bool {
        self.released.load(Ordering::Acquire) && state.users.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_waiter_reuses_its_listener_after_delivering_a_lease() {
        let heap = HeapBufferPool::new(2, 8).unwrap();
        let first = heap.try_acquire().unwrap();
        let second = heap.try_acquire().unwrap();
        let mut acquisition = Acquisition::new(heap);
        assert!(acquisition.next(Waker::noop()).is_none());
        drop(first);
        let first = acquisition.next(Waker::noop()).unwrap();
        drop(second);
        let second = acquisition.next(Waker::noop()).unwrap();
        assert_eq!(first.as_ref().len(), 8);
        assert_eq!(second.as_ref().len(), 8);
        assert!(acquisition.next(Waker::noop()).is_none());
    }
}
