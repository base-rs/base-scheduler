// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use super::{
    registered_file::RegisteredFile,
    ring_binding::{RingBinding, RingCache},
};
use std::{any::Any, sync::Mutex};

/// Reusable file request storage associated with one io_uring selector group. The request can select a different worker between completed operations.
pub struct IoUringFileRegistration {
    owner: Weak<DriverGroup>,
    shared: Arc<Shared>,
    cached: Option<Box<dyn Any + Send>>,
    running: Option<RingBinding<u64>>,
    file: Arc<StdFile>,
    files: RingCache<Arc<RegisteredFile>>,
    retiring: Vec<Arc<RegisteredFile>>,
    closed: bool,
    failed: bool,
}

enum Completion {
    Request(io::Result<usize>, Box<dyn Any + Send>),
    Rejected(Box<dyn Any + Send>),
    Failed,
}

struct Shared {
    result: Mutex<Option<Completion>>,
    ready: AtomicBool,
    waker: atomic_waker::AtomicWaker,
}

impl Shared {
    fn publish(&self, result: Completion) {
        *self.result.lock().unwrap() = Some(result);
        self.ready.store(true, Ordering::Release);
        self.waker.wake();
    }

    fn take(&self) -> Completion {
        self.result.lock().unwrap().take().unwrap()
    }
}

struct FileRequest<B: IoBuf> {
    identifier: u64,
    permit: Option<Permit>,
    file: Option<Arc<RegisteredFile>>,
    offset: Option<u64>,
    buffer: Option<B>,
    pointer: fn(&mut B) -> *mut u8,
    write: bool,
    reply: Cell<Option<Arc<Shared>>>,
    cancelling: Cell<bool>,
}

impl<B: IoBuf> PendingRequest for FileRequest<B> {
    fn identifier(&self) -> u64 {
        self.identifier
    }

    fn prepare(&mut self, ring: &io_uring::Submitter<'_>) -> io::Result<()> {
        self.file.as_ref().unwrap().prepare(ring);
        Ok(())
    }

    fn entry(&mut self) -> squeue::Entry {
        let buffer = self.buffer.as_mut().unwrap();
        let length = buffer.as_ref().len().min(i32::MAX as usize) as u32;
        let pointer = (self.pointer)(buffer);
        let file = self.file.as_ref().unwrap();
        let descriptor = types::Fd(file.file.as_raw_fd());
        let fixed = file.index();
        let offset = self.offset.unwrap_or(u64::MAX);
        let entry = if self.write {
            match fixed {
                Some(fixed) => opcode::Write::new(fixed, pointer, length)
                    .offset(offset)
                    .build(),
                None => opcode::Write::new(descriptor, pointer, length)
                    .offset(offset)
                    .build(),
            }
        } else {
            match fixed {
                Some(fixed) => opcode::Read::new(fixed, pointer, length)
                    .offset(offset)
                    .build(),
                None => opcode::Read::new(descriptor, pointer, length)
                    .offset(offset)
                    .build(),
            }
        };
        entry.user_data(self.identifier)
    }

    fn finish(mut self: Box<Self>, result: io::Result<usize>) {
        let shared = self.reply.take().unwrap();
        // Release kernel-accessible resources and capacity before publishing retirement. The cached box has no back-reference to Shared.
        drop(self.file.take());
        drop(self.permit.take());
        shared.publish(Completion::Request(result, self));
    }

    fn begin_cancellation(&self) -> bool {
        !self.cancelling.replace(true)
    }

    fn take_reply(&self) -> Option<Box<dyn Send>> {
        self.reply
            .take()
            .map(|shared| Box::new(Failure(shared)) as Box<dyn Send>)
    }
}

impl<B: IoBuf> Drop for FileRequest<B> {
    fn drop(&mut self) {
        drop(self.file.take());
        drop(self.permit.take());
        // Queued commands may be destroyed before reaching the ring. Published requests with unknown retirement are deliberately leaked by InFlight instead.
        if let Some(shared) = self.reply.take() {
            shared.publish(Completion::Rejected(Box::new(self.buffer.take().unwrap())));
        }
    }
}

struct Failure(Arc<Shared>);
impl Drop for Failure {
    fn drop(&mut self) {
        self.0.publish(Completion::Failed);
    }
}

impl IoUringSelector {
    fn check_file_monitor(
        &self,
        monitor: &FileMonitor<NativeFile, IoUringFileRegistration>,
    ) -> io::Result<()> {
        if !std::ptr::eq(
            monitor.registration().owner.as_ptr(),
            Arc::as_ptr(&self.group),
        ) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "file monitor belongs to a different selector",
            ));
        }
        if !monitor.file().owns_native(&monitor.registration().file) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "file monitor registration belongs to a different file",
            ));
        }
        Ok(())
    }

    async fn file_monitor_operation<B: IoBuf>(
        &self,
        monitor: &mut FileMonitor<NativeFile, IoUringFileRegistration>,
        buffer: B,
        offset: Option<u64>,
        write: bool,
        pointer: fn(&mut B) -> *mut u8,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        match self
            .file_monitor_operation_submit(monitor, buffer, offset, write, pointer, cancellation)
            .await
        {
            Ok(completion) => completion.await,
            Err((error, buffer)) => (Err(error), buffer),
        }
    }

    async fn file_monitor_operation_submit<B: IoBuf>(
        &self,
        monitor: &mut FileMonitor<NativeFile, IoUringFileRegistration>,
        buffer: B,
        offset: Option<u64>,
        write: bool,
        pointer: fn(&mut B) -> *mut u8,
        cancellation: &Cancellation,
    ) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
        let check = cancellation
            .check()
            .map_err(OperationError::from)
            .and_then(|()| self.check_file_monitor(monitor).map_err(Into::into));
        if let Err(error) = check {
            return Err((error, buffer));
        }
        if offset.is_some_and(|offset| offset > i64::MAX as u64) {
            return Err((
                io::Error::new(io::ErrorKind::InvalidInput, "file offset exceeds i64::MAX").into(),
                buffer,
            ));
        }
        let registration = monitor.registration_mut();
        if registration.closed {
            return Err((
                io::Error::new(io::ErrorKind::BrokenPipe, "file monitor is closed").into(),
                buffer,
            ));
        }
        if registration.running.is_some() {
            if !registration.shared.ready.load(Ordering::Acquire) {
                return Err((
                    io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "file monitor operation has not retired",
                    )
                    .into(),
                    buffer,
                ));
            }
            registration.running = None;
            // Abandonment forfeits the old buffer to retirement. Dispose of it without a completion lock held.
            match registration.shared.take() {
                Completion::Request(_, cached) => registration.cached = Some(cached),
                Completion::Rejected(buffer) => drop(buffer),
                Completion::Failed => {
                    registration.closed = true;
                    registration.failed = true;
                    return Err((
                        io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "selector failed with an operation in flight",
                        )
                        .into(),
                        buffer,
                    ));
                }
            }
        }
        if let Err(error) = self.check_open() {
            return Err((error.into(), buffer));
        }
        let shared = Arc::clone(&registration.shared);
        let controller = self.route();
        let permit = match controller.capacity.acquire(cancellation).await {
            Ok(permit) => permit,
            Err(error) => {
                return Err((
                    operation_result::<usize>(Err(error), cancellation).unwrap_err(),
                    buffer,
                ));
            }
        };
        let identifier = match controller.next_identifier() {
            Ok(identifier) => identifier,
            Err(error) => return Err((error.into(), buffer)),
        };
        let cached = monitor.registration_mut().cached.take();
        let mut request = cached
            .and_then(|cached| cached.downcast::<FileRequest<B>>().ok())
            .unwrap_or_else(|| {
                Box::new(FileRequest {
                    identifier,
                    permit: None,
                    file: None,
                    offset,
                    buffer: None,
                    pointer,
                    write,
                    reply: Cell::new(None),
                    cancelling: Cell::new(false),
                })
            });
        drop(request.buffer.take());
        request.identifier = identifier;
        request.permit = Some(permit);
        let registration = monitor.registration_mut();
        // Keep only the current registration; unfinished or failed releases remain observable until close.
        registration.retiring.retain(|file| !file.released());
        if !registration.files.matches(&controller) {
            if let Some(previous) = registration.files.take_current() {
                previous.retire();
                registration.retiring.push(previous);
            }
        }
        let file = &registration.file;
        request.file = Some(Arc::clone(
            registration
                .files
                .get_or_insert(&controller, || RegisteredFile::new(&controller, file)),
        ));
        request.offset = offset;
        request.buffer = Some(buffer);
        request.pointer = pointer;
        request.write = write;
        request.reply.set(Some(Arc::clone(&shared)));
        request.cancelling.set(false);
        shared.ready.store(false, Ordering::Relaxed);
        monitor.registration_mut().running = Some(RingBinding::new(&controller, identifier));
        if let Err(mpsc::SendError(Command::Submit(request))) =
            controller.send(Command::Submit(request))
        {
            request.finish(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "io_uring selector has exited",
            )));
        }
        let mut cancel = CancelOnDrop {
            controller,
            identifier,
            armed: true,
        };
        Ok(async move {
            let mut cancelled = pin!(cancellation.cancelled());
            poll_fn(|context| {
                shared.waker.register(context.waker());
                if shared.ready.load(Ordering::Acquire) {
                    return Poll::Ready(());
                }
                if cancel.armed && cancelled.as_mut().poll(context).is_ready() {
                    cancel.request();
                }
                Poll::Pending
            })
            .await;
            cancel.armed = false;
            monitor.registration_mut().running = None;
            match shared.take() {
                Completion::Request(result, cached) => {
                    let mut request = cached.downcast::<FileRequest<B>>().ok().unwrap();
                    let buffer = request.buffer.take().unwrap();
                    monitor.registration_mut().cached = Some(request);
                    (operation_result(result, cancellation), buffer)
                }
                Completion::Rejected(buffer) => (
                    Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "selector closed before submission",
                    )
                    .into()),
                    *buffer.downcast::<B>().ok().unwrap(),
                ),
                Completion::Failed => {
                    monitor.registration_mut().closed = true;
                    monitor.registration_mut().failed = true;
                    panic!("io_uring selector failed with an operation in flight");
                }
            }
        })
    }
}

impl FileMonitorSubmission for IoUringSelector {
    async fn file_monitor_read_submit<B: IoBufMut>(
        &self,
        monitor: &mut FileMonitor<NativeFile, IoUringFileRegistration>,
        buffer: B,
        cancellation: &Cancellation,
    ) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
        self.file_monitor_operation_submit(
            monitor,
            buffer,
            None,
            false,
            |buffer| buffer.as_mut().as_mut_ptr(),
            cancellation,
        )
        .await
    }

    async fn file_monitor_write_submit<B: IoBuf>(
        &self,
        monitor: &mut FileMonitor<NativeFile, IoUringFileRegistration>,
        buffer: B,
        cancellation: &Cancellation,
    ) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
        self.file_monitor_operation_submit(
            monitor,
            buffer,
            None,
            true,
            |buffer| buffer.as_ref().as_ptr().cast_mut(),
            cancellation,
        )
        .await
    }

    async fn file_monitor_read_at_submit<B: IoBufMut>(
        &self,
        monitor: &mut FileMonitor<NativeFile, IoUringFileRegistration>,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
        self.file_monitor_operation_submit(
            monitor,
            buffer,
            Some(offset),
            false,
            |buffer| buffer.as_mut().as_mut_ptr(),
            cancellation,
        )
        .await
    }

    async fn file_monitor_write_at_submit<B: IoBuf>(
        &self,
        monitor: &mut FileMonitor<NativeFile, IoUringFileRegistration>,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
        self.file_monitor_operation_submit(
            monitor,
            buffer,
            Some(offset),
            true,
            |buffer| buffer.as_ref().as_ptr().cast_mut(),
            cancellation,
        )
        .await
    }
}

impl FileMonitoring for IoUringSelector {
    type FileRegistration = IoUringFileRegistration;

    fn file_monitor(
        &self,
        file: &NativeFile,
    ) -> io::Result<FileMonitor<NativeFile, IoUringFileRegistration>> {
        self.fallback.file_monitor(file).map(|monitor| {
            monitor.map_registration(|()| IoUringFileRegistration {
                owner: Arc::downgrade(&self.group),
                cached: None,
                running: None,
                file: retain_file(file),
                files: RingCache::default(),
                retiring: Vec::new(),
                closed: false,
                failed: false,
                shared: Arc::new(Shared {
                    result: Mutex::new(None),
                    ready: AtomicBool::new(false),
                    waker: atomic_waker::AtomicWaker::new(),
                }),
            })
        })
    }

    async fn file_monitor_read<B: IoBufMut>(
        &self,
        monitor: &mut FileMonitor<NativeFile, IoUringFileRegistration>,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.file_monitor_operation(
            monitor,
            buffer,
            None,
            false,
            |buffer| buffer.as_mut().as_mut_ptr(),
            cancellation,
        )
        .await
    }
    async fn file_monitor_write<B: IoBuf>(
        &self,
        monitor: &mut FileMonitor<NativeFile, IoUringFileRegistration>,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.file_monitor_operation(
            monitor,
            buffer,
            None,
            true,
            |buffer| buffer.as_ref().as_ptr().cast_mut(),
            cancellation,
        )
        .await
    }
    async fn file_monitor_read_at<B: IoBufMut>(
        &self,
        monitor: &mut FileMonitor<NativeFile, IoUringFileRegistration>,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.file_monitor_operation(
            monitor,
            buffer,
            Some(offset),
            false,
            |buffer| buffer.as_mut().as_mut_ptr(),
            cancellation,
        )
        .await
    }
    async fn file_monitor_write_at<B: IoBuf>(
        &self,
        monitor: &mut FileMonitor<NativeFile, IoUringFileRegistration>,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.file_monitor_operation(
            monitor,
            buffer,
            Some(offset),
            true,
            |buffer| buffer.as_ref().as_ptr().cast_mut(),
            cancellation,
        )
        .await
    }

    async fn file_monitor_close(
        &self,
        monitor: &mut FileMonitor<NativeFile, IoUringFileRegistration>,
    ) -> io::Result<()> {
        self.check_file_monitor(monitor)?;
        let registration = monitor.registration_mut();
        registration.closed = true;
        if registration.failed {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "selector failed before file retirement",
            ));
        }
        if let Some(binding) = &registration.running {
            if !registration.shared.ready.load(Ordering::Acquire) {
                let _ = binding.controller.send(Command::Cancel(binding.state));
            }
            poll_fn(|context| {
                registration.shared.waker.register(context.waker());
                if registration.shared.ready.load(Ordering::Acquire) {
                    return Poll::Ready(());
                }
                Poll::Pending
            })
            .await;
            registration.running = None;
            let completion = registration.shared.take();
            if matches!(completion, Completion::Failed) {
                registration.failed = true;
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "selector failed before file retirement",
                ));
            }
            drop(completion);
        }
        drop(registration.cached.take());
        for file in registration.files.values() {
            file.retire();
        }
        for file in registration.files.values() {
            file.closed().await?;
        }
        for file in &registration.retiring {
            file.closed().await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
