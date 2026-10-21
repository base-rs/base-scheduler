// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use std::{any::Any, sync::Mutex};

/// Reusable file request storage associated with one io_uring selector group. The request can select a different worker between completed operations.
pub struct IoUringFileRegistration {
    owner: Weak<DriverGroup>,
    shared: Arc<Shared>,
}

#[derive(Default)]
struct State {
    cached: Option<Box<dyn Any + Send>>,
    // Only never-submitted requests need this separate owner-return path.
    buffer: Option<Box<dyn Any + Send>>,
    result: Option<io::Result<usize>>,
    running: Option<(Arc<Controller>, u64)>,
    closed: bool,
    failed: bool,
}

struct Shared {
    state: Mutex<State>,
    completed: CompletionEvent,
}

struct FileRequest<B: IoBuf> {
    identifier: u64,
    permit: Option<Permit>,
    file: Option<Arc<StdFile>>,
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

    fn entry(&mut self) -> squeue::Entry {
        let buffer = self.buffer.as_mut().unwrap();
        let length = buffer.as_ref().len().min(i32::MAX as usize) as u32;
        let pointer = (self.pointer)(buffer);
        let descriptor = types::Fd(self.file.as_ref().unwrap().as_raw_fd());
        let offset = self.offset.unwrap_or(u64::MAX);
        let entry = if self.write {
            opcode::Write::new(descriptor, pointer, length)
                .offset(offset)
                .build()
        } else {
            opcode::Read::new(descriptor, pointer, length)
                .offset(offset)
                .build()
        };
        entry.user_data(self.identifier)
    }

    fn finish(mut self: Box<Self>, result: io::Result<usize>) {
        let shared = self.reply.take().unwrap();
        // Release kernel-accessible resources and capacity before publishing retirement. The cached box has no back-reference to Shared.
        drop(self.file.take());
        drop(self.permit.take());
        {
            let mut state = shared.state.lock().unwrap();
            state.running = None;
            state.result = Some(result);
            state.cached = Some(self);
        }
        shared.completed.notify(usize::MAX);
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
            {
                let mut state = shared.state.lock().unwrap();
                state.buffer = Some(Box::new(self.buffer.take().unwrap()));
                state.result = Some(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "selector closed before submission",
                )));
                state.running = None;
            }
            shared.completed.notify(usize::MAX);
        }
    }
}

struct Failure(Arc<Shared>);
impl Drop for Failure {
    fn drop(&mut self) {
        self.0.state.lock().unwrap().failed = true;
        self.0.completed.notify(usize::MAX);
    }
}

impl IoUringSelector {
    fn check_file_monitor(
        &self,
        monitor: &FileMonitor<NativeFile, IoUringFileRegistration>,
    ) -> io::Result<()> {
        if !Weak::ptr_eq(&monitor.registration().owner, &Arc::downgrade(&self.group)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "file monitor belongs to a different selector",
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
        let check = cancellation
            .check()
            .map_err(OperationError::from)
            .and_then(|()| self.check_file_monitor(monitor).map_err(Into::into))
            .and_then(|()| self.check_open().map_err(Into::into));
        if let Err(error) = check {
            return (Err(error), buffer);
        }
        if offset.is_some_and(|offset| offset > i64::MAX as u64) {
            return (
                Err(
                    io::Error::new(io::ErrorKind::InvalidInput, "file offset exceeds i64::MAX")
                        .into(),
                ),
                buffer,
            );
        }
        let shared = Arc::clone(&monitor.registration().shared);
        let error = {
            let state = shared.state.lock().unwrap();
            if state.failed {
                Some((
                    io::ErrorKind::BrokenPipe,
                    "selector failed with an operation in flight",
                ))
            } else if state.closed {
                Some((io::ErrorKind::BrokenPipe, "file monitor is closed"))
            } else if state.running.is_some() {
                Some((
                    io::ErrorKind::WouldBlock,
                    "file monitor operation has not retired",
                ))
            } else {
                None
            }
        };
        if let Some((kind, message)) = error {
            return (Err(io::Error::new(kind, message).into()), buffer);
        }
        let controller = self.route();
        let permit = match controller.capacity.acquire(cancellation).await {
            Ok(permit) => permit,
            Err(error) => return (operation_result(Err(error), cancellation), buffer),
        };
        let identifier = match controller.next_identifier() {
            Ok(identifier) => identifier,
            Err(error) => return (Err(error.into()), buffer),
        };
        let (cached, old_buffer, old_result) = {
            let mut state = shared.state.lock().unwrap();
            (
                state.cached.take(),
                state.buffer.take(),
                state.result.take(),
            )
        };
        // Owners and error payloads may have arbitrary destructors. Never destroy them under the completion lock.
        drop((old_buffer, old_result));
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
        request.file = Some(retain_file(monitor.file()));
        request.offset = offset;
        request.buffer = Some(buffer);
        request.pointer = pointer;
        request.write = write;
        request.reply.set(Some(Arc::clone(&shared)));
        request.cancelling.set(false);
        shared.state.lock().unwrap().running = Some((Arc::clone(&controller), identifier));
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
        let mut cancelled = pin!(cancellation.cancelled());
        loop {
            event_listener::listener!(shared.completed => listener);
            let mut listener = pin!(listener);
            poll_fn(|context| {
                let (ready, failed) = {
                    let state = shared.state.lock().unwrap();
                    (state.result.is_some(), state.failed)
                };
                assert!(
                    !failed,
                    "io_uring selector failed with an operation in flight"
                );
                if ready {
                    return Poll::Ready(());
                }
                if cancelled.as_mut().poll(context).is_ready() {
                    cancel.request();
                }
                listener.as_mut().poll(context).map(|_| ())
            })
            .await;
            let completed = {
                let mut state = shared.state.lock().unwrap();
                state
                    .result
                    .take()
                    .map(|result| (result, state.cached.take(), state.buffer.take()))
            };
            if let Some((result, cached, fallback)) = completed {
                cancel.armed = false;
                let buffer = if let Some(cached) = cached {
                    let mut request = cached.downcast::<FileRequest<B>>().ok().unwrap();
                    let buffer = request.buffer.take().unwrap();
                    shared.state.lock().unwrap().cached = Some(request);
                    buffer
                } else {
                    *fallback.unwrap().downcast::<B>().ok().unwrap()
                };
                return (operation_result(result, cancellation), buffer);
            }
        }
    }
}

impl FileMonitoring for IoUringSelector {
    type FileRegistration = IoUringFileRegistration;

    fn file_monitor(
        &self,
        file: &NativeFile,
    ) -> io::Result<FileMonitor<NativeFile, IoUringFileRegistration>> {
        let completed = CompletionEvent::new();
        completed.notify(0);
        Ok(self
            .fallback
            .file_monitor(file)?
            .map_registration(|()| IoUringFileRegistration {
                owner: Arc::downgrade(&self.group),
                shared: Arc::new(Shared {
                    state: Mutex::new(State::default()),
                    completed,
                }),
            }))
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
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        cancellation.check()?;
        self.check_file_monitor(monitor)?;
        let shared = &monitor.registration().shared;
        let running = {
            let mut state = shared.state.lock().unwrap();
            state.closed = true;
            state.running.clone()
        };
        if let Some((controller, identifier)) = running {
            let _ = controller.send(Command::Cancel(identifier));
        }
        let mut cancelled = pin!(cancellation.cancelled());
        loop {
            event_listener::listener!(shared.completed => listener);
            let (running, failed) = {
                let state = shared.state.lock().unwrap();
                (state.running.is_some(), state.failed)
            };
            if failed {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "selector failed before file retirement",
                )
                .into());
            }
            if !running {
                break;
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
        let retired = {
            let mut state = shared.state.lock().unwrap();
            (
                state.cached.take(),
                state.buffer.take(),
                state.result.take(),
            )
        };
        drop(retired);
        Ok(())
    }
}
