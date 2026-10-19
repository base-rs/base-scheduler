// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{IoUringDriver, IoUringOptions};
use core_scheduler::*;
use core_scheduler_selector::{NativeFile, ReadinessSelector, Selector};
use event_listener::Event as CompletionEvent;
use futures_channel::oneshot;
use io_uring::{IoUring, opcode, squeue, types};
use polling::{Event, Events, Poller};
use std::{
    cell::Cell,
    collections::{HashMap, VecDeque},
    fs::File as StdFile,
    future::{Future, poll_fn},
    io::{self, Read, Write},
    os::{fd::AsRawFd, unix::net::UnixStream},
    pin::pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    task::Poll,
    time::Duration,
};
const CANCEL_TAG: u64 = 1 << 63;
const COMMAND_EVENT: usize = 0;
const RING_EVENT: usize = 1;

// Keep syscall failures deterministic in unit tests while production calls the OS directly.
fn syscall<T>(name: &'static str, operation: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    #[cfg(test)]
    {
        tests::syscall(name, operation)
    }
    #[cfg(not(test))]
    {
        let _ = name;
        operation()
    }
}

fn validate_completion_support(nodrop: bool, rw_cur_pos: bool) -> io::Result<()> {
    if !rw_cur_pos {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "io_uring requires shared file cursor support",
        ));
    }
    if nodrop {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "io_uring requires reliable completion overflow handling",
        ))
    }
}

fn validate_operation_support(supported: impl Fn(u8) -> bool) -> io::Result<()> {
    for operation in [
        opcode::Read::CODE,
        opcode::Write::CODE,
        opcode::AsyncCancel::CODE,
    ] {
        if !supported(operation) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "required io_uring operation is unavailable",
            ));
        }
    }
    Ok(())
}

fn discard<T>(value: T) {
    if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(value))) {
        // A destructor panic has no receiving caller; its payload can itself panic on drop.
        std::mem::forget(payload);
    }
}

type RawBufferResult<B> = (io::Result<usize>, B);

enum Resource {
    // None uses the shared OS cursor; Some is a validated explicit offset.
    File(Arc<StdFile>, Option<u64>),
}

struct Request<B: IoBuf = Vec<u8>> {
    identifier: u64,
    _permit: Permit,
    resource: Option<Resource>,
    buffer: Option<B>,
    pointer: fn(&mut B) -> *mut u8,
    write: bool,
    reply: Cell<Option<oneshot::Sender<RawBufferResult<B>>>>,
    cancelling: Cell<bool>,
}

impl<B: IoBuf> Request<B> {
    fn entry(&mut self) -> squeue::Entry {
        let buffer = self.buffer.as_mut().expect("request retains its buffer");
        let length = buffer.as_ref().len().min(i32::MAX as usize) as u32;
        let pointer = (self.pointer)(buffer);
        let entry = match self
            .resource
            .as_ref()
            .expect("request retains its resource")
        {
            Resource::File(file, offset) => {
                let descriptor = types::Fd(file.as_raw_fd());
                if self.write {
                    opcode::Write::new(descriptor, pointer, length)
                        .offset(offset.unwrap_or(u64::MAX))
                        .build()
                } else {
                    opcode::Read::new(descriptor, pointer, length)
                        .offset(offset.unwrap_or(u64::MAX))
                        .build()
                }
            }
        };
        entry.user_data(self.identifier)
    }

    fn finish(mut self, result: io::Result<usize>) {
        let reply = self
            .reply
            .take()
            .expect("unfinished requests retain their completion sender");
        let buffer = self.buffer.take().expect("request returns its buffer once");
        // Release the retained descriptor before waking a caller that may immediately close it.
        drop(self);
        discard(reply.send((result, buffer)));
    }
}

impl<B: IoBuf> Drop for Request<B> {
    fn drop(&mut self) {
        drop(self.resource.take());
        // A submission can race Shutdown and remain in the incoming queue as
        // the selector exits. Such requests have never reached the kernel and
        // can return their buffers normally. InFlight deliberately suppresses
        // this destructor for requests whose completion is unknown.
        if let Some(reply) = self.reply.take() {
            discard(reply.send((
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "selector closed before submission",
                )),
                self.buffer.take().expect("request returns its buffer once"),
            )));
        }
    }
}

// One erased request allocation lets a selector queue heterogeneous owned buffer types. Its typed completion returns the exact original owner.
// Only entry preparation (before publication) and original completion may access the request exclusively. In-flight bookkeeping uses shared access so inline UnsafeCell output can remain kernel-accessible without creating an aliasing mutable reference.
trait PendingRequest: Send {
    fn identifier(&self) -> u64;
    fn entry(&mut self) -> squeue::Entry;
    fn finish(self: Box<Self>, result: io::Result<usize>);
    fn cancelling(&self) -> bool;
    fn set_cancelling(&self);
    fn take_reply(&self) -> Option<Box<dyn Send>>;
}
impl<B: IoBuf> PendingRequest for Request<B> {
    fn identifier(&self) -> u64 {
        self.identifier
    }
    fn entry(&mut self) -> squeue::Entry {
        Request::entry(self)
    }
    fn finish(self: Box<Self>, result: io::Result<usize>) {
        Request::finish(*self, result);
    }
    fn cancelling(&self) -> bool {
        self.cancelling.get()
    }
    fn set_cancelling(&self) {
        self.cancelling.set(true);
    }
    fn take_reply(&self) -> Option<Box<dyn Send>> {
        self.reply
            .take()
            .map(|reply| Box::new(reply) as Box<dyn Send>)
    }
}

/// Only terminal original completions remove requests from this map.
/// On an unexpected selector failure, leak kernel-accessible resources rather
/// than freeing memory whose completion cannot be established.
#[derive(Default)]
struct InFlight(HashMap<u64, Box<dyn PendingRequest>>);

impl Drop for InFlight {
    fn drop(&mut self) {
        let mut replies = Vec::new();
        for (_, request) in self.0.drain() {
            replies.extend(request.take_reply());
            std::mem::forget(request);
        }
        // Wake abandoned callers only after every in-flight buffer is retained.
        drop(replies);
    }
}

enum Command {
    Submit(Box<dyn PendingRequest>),
    Cancel(u64),
    Shutdown,
}

struct Shutdown {
    finished: AtomicBool,
    event: CompletionEvent,
}

pub(crate) struct SignalCompletion(Arc<Shutdown>);
impl Drop for SignalCompletion {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::Release);
        self.0.event.notify(usize::MAX);
    }
}

pub(crate) struct Controller {
    capacity: Arc<Capacity>,
    commands: mpsc::Sender<Command>,
    notification: UnixStream,
    next_identifier: AtomicU64,
    closed: AtomicBool,
    shutdown: Arc<Shutdown>,
}

impl Controller {
    fn notify(&self) {
        // A full pipe already contains a wakeup. Other errors mean the selector
        // has exited; dropping its receivers wakes pending operation futures.
        loop {
            match syscall("notify", || (&self.notification).write(&[1])) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                _ => break,
            }
        }
    }

    pub(crate) fn close(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            self.capacity.close();
            let _ = self.commands.send(Command::Shutdown);
            self.notify();
        }
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.close();
    }
}

struct CancelOnDrop {
    controller: Arc<Controller>,
    identifier: u64,
    armed: bool,
}
impl CancelOnDrop {
    fn request(&mut self) {
        if self.armed {
            let _ = self
                .controller
                .commands
                .send(Command::Cancel(self.identifier));
            self.controller.notify();
            self.armed = false;
        }
    }
}
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.request();
    }
}

struct Capacity {
    state: Mutex<(usize, bool)>,
    available: CompletionEvent,
    limit: usize,
}
struct Permit(Arc<Capacity>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.state.lock().unwrap().0 -= 1;
        self.0.available.notify(usize::MAX);
    }
}
impl Capacity {
    pub(crate) fn close(&self) {
        self.state.lock().unwrap().1 = true;
        self.available.notify(usize::MAX);
    }
    async fn acquire(self: &Arc<Self>, cancellation: &Cancellation) -> io::Result<Permit> {
        let mut cancelled = pin!(cancellation.cancelled());
        loop {
            event_listener::listener!(self.available => listener);
            cancellation
                .check()
                .map_err(|error| io::Error::new(io::ErrorKind::Interrupted, error))?;
            {
                let mut state = self.state.lock().unwrap();
                if state.1 {
                    return Err(io::ErrorKind::BrokenPipe.into());
                }
                if state.0 < self.limit {
                    state.0 += 1;
                    return Ok(Permit(Arc::clone(self)));
                }
            }
            let mut listener = pin!(listener);
            poll_fn(|cx| {
                if cancelled.as_mut().poll(cx).is_ready() {
                    return Poll::Ready(());
                }
                listener.as_mut().poll(cx)
            })
            .await;
        }
    }
}
/// Owned I/O admission endpoint, independent of the executor polling its futures.
///
/// `new` returns a separate driver and starts no ring thread. File transfers use io_uring; other capabilities explicitly use the readiness fallback. Dropping this endpoint closes admission without blocking.
pub struct IoUringSelector {
    controller: Arc<Controller>,
    pub(crate) fallback: Arc<ReadinessSelector>,
}
impl IoUringSelector {
    /// Create an endpoint and its driver. The driver must be run throughout operation and shutdown. Unsupported kernels or restricted ring setup return errors; ring file transfers never silently fall back.
    pub fn new(options: IoUringOptions) -> io::Result<(Self, IoUringDriver)> {
        if options.entries == 0 || options.capacity == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ring entries and request capacity must be nonzero",
            ));
        }
        let fallback = Arc::new(ReadinessSelector::with_options(options.fallback)?);
        let ring = syscall("ring", || IoUring::new(options.entries))?;
        syscall("completion-support", || {
            validate_completion_support(
                ring.params().is_feature_nodrop(),
                ring.params().is_feature_rw_cur_pos(),
            )
        })?;
        let mut probe = io_uring::Probe::new();
        syscall("probe", || ring.submitter().register_probe(&mut probe))?;
        syscall("operation-support", || {
            validate_operation_support(|operation| probe.is_supported(operation))
        })?;
        let (notification, receiver) = syscall("notification-pair", UnixStream::pair)?;
        syscall("notification-mode", || notification.set_nonblocking(true))?;
        syscall("receiver-mode", || receiver.set_nonblocking(true))?;
        let (commands, incoming) = mpsc::channel();
        let shutdown = Arc::new(Shutdown {
            finished: AtomicBool::new(false),
            event: CompletionEvent::new(),
        });
        let capacity = Arc::new(Capacity {
            state: Mutex::new((0, false)),
            available: CompletionEvent::new(),
            limit: options.capacity,
        });
        let controller = Arc::new(Controller {
            capacity,
            commands,
            notification,
            next_identifier: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            shutdown: Arc::clone(&shutdown),
        });
        let reactor = Reactor::new(ring, receiver, incoming)?;
        let driver = IoUringDriver {
            reactor: Some(reactor),
            controller: Arc::clone(&controller),
            fallback: Arc::clone(&fallback),
            completion: Some(SignalCompletion(shutdown)),
        };
        Ok((
            Self {
                controller,
                fallback,
            },
            driver,
        ))
    }

    /// Stop admission and request cancellation of accepted ring work. This does not wait for retirement.
    pub fn close(&self) {
        self.controller.close();
        self.fallback.close();
    }

    /// Close admission and await fallback and ring retirement. Continue driving the separately owned driver while awaiting this method.
    pub async fn shutdown(&self) {
        self.close();
        self.fallback.shutdown().await;
        loop {
            event_listener::listener!(self.controller.shutdown.event => listener);
            if self.controller.shutdown.finished.load(Ordering::Acquire) {
                return;
            }
            listener.await;
        }
    }
    fn check_open(&self) -> io::Result<()> {
        if self.controller.closed.load(Ordering::Acquire)
            || self.controller.shutdown.finished.load(Ordering::Acquire)
        {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "io_uring selector is closed",
            ))
        } else {
            Ok(())
        }
    }

    async fn operation<B: IoBuf>(
        &self,
        resource: Resource,
        buffer: B,
        write: bool,
        pointer: fn(&mut B) -> *mut u8,
        cancellation: &Cancellation,
    ) -> RawBufferResult<B> {
        if cancellation.is_cancelled() {
            return (
                Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    core_scheduler::Cancelled,
                )),
                buffer,
            );
        }
        if let Err(error) = self.check_open() {
            return (Err(error), buffer);
        }
        if matches!(&resource, Resource::File(_, Some(offset)) if *offset > i64::MAX as u64) {
            return (
                Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "file offset exceeds i64::MAX",
                )),
                buffer,
            );
        }
        let permit = match self.controller.capacity.acquire(cancellation).await {
            Ok(permit) => permit,
            Err(error) => return (Err(error), buffer),
        };
        let identifier = match self.next_identifier() {
            Ok(identifier) => identifier,
            Err(error) => return (Err(error), buffer),
        };
        let (reply, result) = oneshot::channel();
        let request = Request {
            identifier,
            _permit: permit,
            resource: Some(resource),
            buffer: Some(buffer),
            pointer,
            write,
            reply: Cell::new(Some(reply)),
            cancelling: Cell::new(false),
        };
        self.submit(Box::new(request), result, cancellation).await
    }

    // `try_update` is not available on the supported Rust 1.85 toolchain.
    #[allow(deprecated)]
    fn next_identifier(&self) -> io::Result<u64> {
        self.controller
            .next_identifier
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |identifier| {
                (identifier < CANCEL_TAG - 1).then_some(identifier + 1)
            })
            .map_err(|_| io::Error::other("io_uring operation identifiers exhausted"))
    }

    async fn submit<T>(
        &self,
        request: Box<dyn PendingRequest>,
        result: oneshot::Receiver<T>,
        cancellation: &Cancellation,
    ) -> T {
        let identifier = request.identifier();
        if let Err(mpsc::SendError(Command::Submit(request))) =
            self.controller.commands.send(Command::Submit(request))
        {
            request.finish(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "io_uring selector has exited",
            )));
        }

        let mut cancel_request = CancelOnDrop {
            controller: Arc::clone(&self.controller),
            identifier,
            armed: true,
        };
        self.controller.notify();
        // A disconnected reply is an unexpected selector failure. Resources whose kernel completion cannot be established are deliberately retained.
        let mut result = pin!(result);
        let mut cancelled = pin!(cancellation.cancelled());
        let result = poll_fn(|context| {
            // Original completion wins the race. Cancellation never releases resources; continue waiting for the original CQE.
            if let Poll::Ready(result) = result.as_mut().poll(context) {
                return Poll::Ready(result);
            }
            if cancel_request.armed && cancelled.as_mut().poll(context).is_ready() {
                cancel_request.request();
            }
            Poll::Pending
        })
        .await
        .expect("io_uring selector failed with an operation in flight");
        cancel_request.armed = false;
        result
    }
}

fn operation_result<T>(result: io::Result<T>, cancellation: &Cancellation) -> OperationResult<T> {
    match result {
        Err(error)
            if cancellation.is_cancelled()
                && (error.kind() == io::ErrorKind::Interrupted
                    || error.raw_os_error() == Some(libc::ECANCELED)) =>
        {
            Err(core_scheduler::Cancelled.into())
        }
        result => result.map_err(Into::into),
    }
}
impl FileRead for IoUringSelector {
    async fn file_read<B: IoBufMut>(
        &self,
        file: &Self::File,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        let (result, buffer) = self
            .operation(
                Resource::File(retain_file(file), None),
                buffer,
                false,
                |buffer| buffer.as_mut().as_mut_ptr(),
                cancellation,
            )
            .await;
        (operation_result(result, cancellation), buffer)
    }
}

impl FileWrite for IoUringSelector {
    async fn file_write<B: IoBuf>(
        &self,
        file: &Self::File,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        let (result, buffer) = self
            .operation(
                Resource::File(retain_file(file), None),
                buffer,
                true,
                |buffer| buffer.as_ref().as_ptr().cast_mut(),
                cancellation,
            )
            .await;
        (operation_result(result, cancellation), buffer)
    }
}

impl FileReadAt for IoUringSelector {
    async fn file_read_at<B: IoBufMut>(
        &self,
        file: &Self::File,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        let (result, buffer) = self
            .operation(
                Resource::File(retain_file(file), Some(offset)),
                buffer,
                false,
                |buffer| buffer.as_mut().as_mut_ptr(),
                cancellation,
            )
            .await;
        (operation_result(result, cancellation), buffer)
    }
}

impl FileWriteAt for IoUringSelector {
    async fn file_write_at<B: IoBuf>(
        &self,
        file: &Self::File,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        let (result, buffer) = self
            .operation(
                Resource::File(retain_file(file), Some(offset)),
                buffer,
                true,
                |buffer| buffer.as_ref().as_ptr().cast_mut(),
                cancellation,
            )
            .await;
        (operation_result(result, cancellation), buffer)
    }
}

fn retain_file(file: &NativeFile) -> Arc<StdFile> {
    // SAFETY: Requests retain this owner until original completion, never change descriptor flags, and release it before waking completion consumers. No native handle escapes this backend.
    unsafe { file.retain_native() }
}
impl Drop for IoUringSelector {
    fn drop(&mut self) {
        self.close();
    }
}
impl Selector for IoUringSelector {
    fn close(&self) {
        self.close();
    }
    async fn shutdown(&self) {
        self.shutdown().await;
    }
}
pub(crate) struct Reactor {
    // The poller is destroyed before its registered sources on error/unwind.
    poller: Poller,
    ring: IoUring,
    notification: UnixStream,
    incoming: mpsc::Receiver<Command>,
    pending: VecDeque<Box<dyn PendingRequest>>,
    in_flight: InFlight,
    cancellations: VecDeque<u64>,
    closing: bool,
    events: Events,
}

impl Reactor {
    fn new(
        ring: IoUring,
        notification: UnixStream,
        incoming: mpsc::Receiver<Command>,
    ) -> io::Result<Self> {
        let reactor = Self {
            poller: syscall("poller", Poller::new)?,
            ring,
            notification,
            incoming,
            pending: VecDeque::new(),
            in_flight: InFlight::default(),
            cancellations: VecDeque::new(),
            closing: false,
            events: Events::new(),
        };
        // SAFETY: Reactor owns both sources and deletes their registrations in
        // Drop, before the poller and sources are destroyed.
        unsafe {
            syscall("add-notification", || {
                reactor
                    .poller
                    .add(&reactor.notification, Event::readable(COMMAND_EVENT))
            })?;
            syscall("add-ring", || {
                reactor
                    .poller
                    .add(&reactor.ring, Event::readable(RING_EVENT))
            })?;
        }
        Ok(reactor)
    }

    fn cancel(&mut self, identifier: u64) {
        if let Some(position) = self
            .pending
            .iter()
            .position(|request| request.identifier() == identifier)
        {
            let request = self
                .pending
                .remove(position)
                .expect("pending queue is unchanged since finding this position");
            request.finish(Err(io::ErrorKind::Interrupted.into()));
        } else if let Some(request) = self.in_flight.0.get(&identifier) {
            if !request.cancelling() {
                request.set_cancelling();
                self.cancellations.push_back(identifier);
            }
        }
    }

    pub(crate) fn turn(&mut self, timeout: Option<Duration>) -> io::Result<bool> {
        let mut progressed = false;
        // Bound command processing so a busy producer cannot starve completions.
        let mut command_batch_full = true;
        for _ in 0..256 {
            match self.incoming.try_recv() {
                Ok(Command::Submit(request)) if !self.closing => {
                    progressed = true;
                    self.pending.push_back(request)
                }
                Ok(Command::Submit(request)) => {
                    progressed = true;
                    request.finish(Err(io::ErrorKind::Interrupted.into()))
                }
                Ok(Command::Cancel(identifier)) => {
                    progressed = true;
                    self.cancel(identifier);
                }
                Ok(Command::Shutdown) => {
                    progressed = true;
                    self.closing = true;
                    for request in self.pending.drain(..) {
                        request.finish(Err(io::ErrorKind::Interrupted.into()));
                    }
                    for request in self.in_flight.0.values() {
                        if !request.cancelling() {
                            request.set_cancelling();
                            self.cancellations.push_back(request.identifier());
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => {
                    command_batch_full = false;
                    break;
                }
            }
        }

        for completion in self.ring.completion() {
            progressed = true;
            let identifier = completion.user_data();
            if identifier & CANCEL_TAG != 0 {
                continue;
            }
            // Every untagged CQE is the single original completion of a
            // published request. Only this path removes it from InFlight.
            let request = self
                .in_flight
                .0
                .remove(&identifier)
                .expect("original completion retains its in-flight request");
            let result = completion.result();
            request.finish(if result < 0 {
                Err(io::Error::from_raw_os_error(-result))
            } else {
                Ok(result as usize)
            });
        }

        {
            let mut submissions = self.ring.submission();
            while let Some(&identifier) = self.cancellations.front() {
                let entry = opcode::AsyncCancel::new(identifier)
                    .build()
                    .user_data(identifier | CANCEL_TAG);
                // SAFETY: cancellation entries reference only a never-reused
                // numeric operation identifier, not userspace memory.
                if unsafe { submissions.push(&entry) }.is_err() {
                    break;
                }
                self.cancellations.pop_front();
            }
            while !submissions.is_full() {
                let Some(request) = self.pending.pop_front() else {
                    break;
                };
                let identifier = request.identifier();
                let request = self.in_flight.0.entry(identifier).or_insert(request);
                let entry = request.entry();
                // SAFETY: InFlight owns the buffer allocation and descriptor
                // before publication. Neither is released or accessed until
                // the original CQE, even when cancellation completes first.
                // A failed push does not publish the entry.
                unsafe { submissions.push(&entry) }
                    .expect("submission capacity was checked and the local queue has not changed");
            }
        }

        match syscall("submit", || self.ring.submit()) {
            Err(error)
                if error.kind() == io::ErrorKind::Interrupted
                    || matches!(error.raw_os_error(), Some(libc::EBUSY | libc::EAGAIN)) =>
            {
                return Ok(false);
            }
            result => {
                result?;
            }
        }

        if self.closing && self.in_flight.0.is_empty() && self.pending.is_empty() {
            return Ok(true);
        }

        // Bound notification reads too: continuous producers must not monopolize a turn. Unread bytes remain readable after rearming, and any observed notification makes this turn's wait nonblocking.
        let mut notified = false;
        let mut bytes = [0; 256];
        for _ in 0..16 {
            match syscall("notification-read", || self.notification.read(&mut bytes)) {
                Ok(0) => unreachable!(
                    "the driver retains the private notification writer until this reactor is destroyed"
                ),
                Ok(_) => notified = true,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
        syscall("modify-notification", || {
            self.poller
                .modify(&self.notification, Event::readable(COMMAND_EVENT))
        })?;
        syscall("modify-ring", || {
            self.poller.modify(&self.ring, Event::readable(RING_EVENT))
        })?;
        self.events.clear();
        let result = syscall("wait", || {
            self.poller.wait(
                &mut self.events,
                if progressed
                    || notified
                    || command_batch_full
                    || !self.pending.is_empty()
                    || !self.ring.submission().is_empty()
                {
                    Some(Duration::ZERO)
                } else {
                    timeout
                },
            )
        });
        if let Err(error) = result {
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
        Ok(false)
    }
}

impl Drop for Reactor {
    fn drop(&mut self) {
        let _ = self.poller.delete(&self.notification);
        let _ = self.poller.delete(&self.ring);
    }
}

impl std::task::Wake for Controller {
    fn wake(self: Arc<Self>) {
        self.notify();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.notify();
    }
}

#[cfg(test)]
mod tests;
