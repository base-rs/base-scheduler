// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::io_uring_selector::{Command, Controller, DriverGroup, provided_buffers::WorkerBuffers};
use core_scheduler::*;
use std::{
    future::{Future, poll_fn},
    io,
    pin::pin,
    sync::{Arc, Mutex, Weak},
};

pub(crate) const RECEIVE_METADATA: usize = 16 + std::mem::size_of::<libc::sockaddr_storage>();

/// One bounded payload budget shared by the selector's workers. Provided-buffer ring metadata is allocated per worker on first use; payload allocations are never multiplied by the worker count.
#[derive(Clone)]
pub struct IoUringBufferPool(pub(crate) Arc<SharedPool>);

pub(crate) struct SharedPool {
    pub(crate) heap: HeapBufferPool,
    pub(crate) options: BufferPoolOptions,
    pub(crate) selector: Weak<DriverGroup>,
    pub(crate) workers: Mutex<Vec<Weak<WorkerBuffers>>>,
}

impl IoUringBufferPool {
    pub(crate) fn new(group: &Arc<DriverGroup>, options: BufferPoolOptions) -> io::Result<Self> {
        // Metadata precedes recvmsg's payload. Round its reserved prefix up so both ordinary leases and completed receive payloads satisfy the requested alignment.
        let alignment = options.alignment;
        let prefix = if alignment.is_power_of_two() {
            RECEIVE_METADATA
                .checked_add(alignment - 1)
                .map(|value| value & !(alignment - 1))
        } else {
            None
        };
        let size = prefix.and_then(|prefix| options.buffer_size.checked_add(prefix));
        if options.buffers == 0
            || options.buffer_size == 0
            || size.is_none_or(|size| size > i32::MAX as usize)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "provided-buffer pools require nonempty storage fitting the receive ABI",
            ));
        }
        let heap = HeapBufferPool::with_options(BufferPoolOptions {
            buffer_size: size.unwrap(),
            ..options
        })?;
        Ok(Self(Arc::new(SharedPool {
            heap,
            options,
            selector: Arc::downgrade(group),
            workers: Mutex::new(Vec::new()),
        })))
    }

    pub(crate) fn worker(&self, controller: &Arc<Controller>) -> io::Result<Arc<WorkerBuffers>> {
        let mut workers = self.0.workers.lock().unwrap();
        workers.retain(|worker| worker.strong_count() != 0);
        if let Some(worker) = workers
            .iter()
            .filter_map(Weak::upgrade)
            .find(|worker| Weak::ptr_eq(&worker.controller, &Arc::downgrade(controller)))
        {
            return Ok(worker);
        }
        let worker = WorkerBuffers::new(Arc::clone(&self.0), controller)?;
        workers.push(Arc::downgrade(&worker));
        Ok(worker)
    }
}

impl SharedPool {
    /// Only the pressure path coordinates workers. Each donor cancels and drains its original receives before making unconsumed storage available again.
    pub(crate) fn reclaim(&self, except: Option<&Arc<WorkerBuffers>>) {
        let workers: Vec<_> = self
            .workers
            .lock()
            .unwrap()
            .iter()
            .filter_map(Weak::upgrade)
            .collect();
        for worker in workers {
            if except.is_some_and(|except| Arc::ptr_eq(except, &worker)) {
                continue;
            }
            if worker.request_reclaim() {
                if let Some(controller) = worker.controller.upgrade() {
                    let _ = controller.send(Command::PoolReclaim(worker));
                }
            }
        }
    }
}

impl BufferPool for IoUringBufferPool {
    type Buffer = crate::IoUringBuffer;
    fn capacity(&self) -> usize {
        self.0.options.buffers
    }
    fn buffer_size(&self) -> usize {
        self.0.options.buffer_size
    }
    fn alignment(&self) -> usize {
        self.0.options.alignment
    }
    fn try_acquire(&self) -> Option<Self::Buffer> {
        let buffer = self.0.heap.try_acquire();
        if buffer.is_none() {
            self.0.reclaim(None);
        }
        buffer.map(|buffer| crate::IoUringBuffer::new(buffer, 0, self.buffer_size()))
    }
    async fn acquire(&self, cancellation: &Cancellation) -> Result<Self::Buffer, Cancelled> {
        cancellation.check()?;
        if let Some(buffer) = self.try_acquire() {
            return Ok(buffer);
        }
        let mut acquiring = pin!(self.0.heap.acquire(cancellation));
        poll_fn(|context| {
            let result = acquiring.as_mut().poll(context);
            if result.is_pending() {
                // A previously returned slot may have been published by another worker before this waiter could claim it.
                self.0.reclaim(None);
            }
            result
        })
        .await
        .map(|buffer| crate::IoUringBuffer::new(buffer, 0, self.buffer_size()))
    }
}
