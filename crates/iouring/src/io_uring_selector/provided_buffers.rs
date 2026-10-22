// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Worker-local publication of leases from one shared payload budget. Only driver calls access registered rings; the mutex also permits cold pressure requests from other threads.

use super::*;
use crate::io_uring_buffer_pool::{RECEIVE_METADATA, SharedPool};
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
    group: multishot::pool::Group,
    pointer: NonNull<types::BufRingEntry>,
    layout: Layout,
    entries: u16,
    registered: bool,
}
// SAFETY: The allocation is stable. Registered access is confined to its owning driver; other threads only inspect worker state under its mutex.
unsafe impl Send for Ring {}

impl Ring {
    fn new(controller: &Controller, buffers: usize) -> io::Result<Self> {
        let group = multishot::pool::Group::new(controller)?;
        let entries = buffers.min(8).next_power_of_two() as u16;
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
        })
    }
    fn register(&mut self, submitter: &io_uring::Submitter<'_>) -> io::Result<()> {
        if !self.registered {
            syscall("register-buffer-ring", || {
                // SAFETY: WorkerState retains this page-aligned ring and every published lease until all original receives retire and registration is removed.
                unsafe {
                    submitter.register_buf_ring_with_flags(
                        self.pointer.as_ptr() as u64,
                        self.entries,
                        self.group.identifier,
                        0,
                    )
                }
            })?;
            self.registered = true;
        }
        Ok(())
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
    users: Vec<u64>,
    active: usize,
    available: usize,
    demand: bool,
    reclaiming: bool,
    acquisition: Option<Acquisition>,
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
    pool: Arc<SharedPool>,
    state: Mutex<WorkerState>,
    reclaim_requested: AtomicBool,
}
impl WorkerBuffers {
    pub(crate) fn new(
        pool: Arc<SharedPool>,
        controller: &Arc<Controller>,
    ) -> io::Result<Arc<Self>> {
        let ring = Ring::new(controller, pool.options.buffers)?;
        let buffers = (0..ring.entries).map(|_| None).collect();
        Ok(Arc::new(Self {
            controller: Arc::downgrade(controller),
            pool,
            state: Mutex::new(WorkerState {
                ring,
                buffers,
                users: Vec::new(),
                active: 0,
                available: 0,
                demand: false,
                reclaiming: false,
                acquisition: None,
            }),
            reclaim_requested: AtomicBool::new(false),
        }))
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
    pub(super) fn group(&self) -> u16 {
        self.state.lock().unwrap().ring.group.identifier
    }
    pub(super) fn has_buffers(&self) -> bool {
        self.state.lock().unwrap().available != 0
    }
    pub(super) fn reclaiming(&self) -> bool {
        self.state.lock().unwrap().reclaiming
    }
    pub(super) fn demand(&self) {
        self.state.lock().unwrap().demand = true;
    }
    pub(super) fn submitted(&self) {
        self.state.lock().unwrap().active += 1;
    }
    pub(super) fn completed(&self) {
        self.state.lock().unwrap().active -= 1;
    }
    pub(super) fn take(&self, identifier: u16) -> BufferLease {
        let mut state = self.state.lock().unwrap();
        let buffer = state.buffers[identifier as usize]
            .take()
            .expect("a selected ID owns its published lease");
        state.available -= 1;
        buffer
    }
    pub(crate) fn request_reclaim(&self) -> bool {
        self.has_buffers() && !self.reclaim_requested.swap(true, Ordering::AcqRel)
    }
    pub(super) fn begin_reclaim(&self) {
        self.state.lock().unwrap().reclaiming = true;
    }
    pub(super) fn empty(&self) -> bool {
        self.state.lock().unwrap().users.is_empty()
    }
    pub(super) fn publish_offset(&self) -> usize {
        let alignment = self.pool.options.alignment;
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
            assert_eq!(
                state.active, 0,
                "the final receive retires before releasing publication"
            );
            state.ring.unregister(submitter)?;
            for buffer in &mut state.buffers {
                drop(buffer.take());
            }
            state.available = 0;
            state.acquisition = None;
        }
        Ok(())
    }
    /// Drive the cold refill/rebalance path on the ring owner. Returns true when dormant receives may retry.
    pub(super) fn service(
        self: &Arc<Self>,
        submitter: &io_uring::Submitter<'_>,
    ) -> io::Result<bool> {
        let mut state = self.state.lock().unwrap();
        if state.users.is_empty() {
            state.ring.unregister(submitter)?;
            for buffer in &mut state.buffers {
                drop(buffer.take());
            }
            state.available = 0;
            state.acquisition = None;
            return Ok(false);
        }
        if state.reclaiming {
            if state.active != 0 {
                return Ok(false);
            }
            state.ring.unregister(submitter)?;
            for buffer in &mut state.buffers {
                drop(buffer.take());
            }
            state.available = 0;
            state.demand = false;
            state.reclaiming = false;
            state.acquisition = None;
            self.reclaim_requested.store(false, Ordering::Release);
            return Ok(true);
        }
        if !state.demand {
            return Ok(false);
        }
        state.ring.register(submitter)?;
        let offset = self.publish_offset();
        for index in 0..state.buffers.len() {
            if state.buffers[index].is_some() {
                continue;
            }
            let buffer = self.pool.heap.try_acquire().or_else(|| {
                state.acquisition.as_mut().and_then(|acquisition| {
                    let controller = self
                        .controller
                        .upgrade()
                        .expect("a driven pool retains its controller");
                    acquisition.next(&Waker::from(controller))
                })
            });
            let Some(mut buffer) = buffer else {
                break;
            };
            state.ring.publish(index, &mut buffer, offset);
            state.buffers[index] = Some(buffer);
            state.available += 1;
        }
        let available = state.available != 0;
        if !available {
            if state.acquisition.is_none() {
                let mut acquisition = Acquisition::new(self.pool.heap.clone());
                let controller = self
                    .controller
                    .upgrade()
                    .expect("a driven pool retains its controller");
                if let Some(mut buffer) = acquisition.next(&Waker::from(controller)) {
                    state.ring.publish(0, &mut buffer, offset);
                    state.buffers[0] = Some(buffer);
                    state.available += 1;
                }
                state.acquisition = Some(acquisition);
            }
            drop(state);
            // A capacity wake does not reserve a lease. Another worker can publish the returned slot before this waiter runs, so exhausted retries must recall peers again.
            self.pool.reclaim(Some(self));
            return Ok(self.has_buffers());
        }
        Ok(available)
    }
}
