// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use std::sync::Mutex;
use std::task::Waker;

pub(super) const MONITOR_TAG: u64 = 1;
const READY: u64 = 1;
const SHUTDOWN: u64 = 2;
const NO_SLOT: usize = usize::MAX;
type State = SocketMonitor<NativeSocket, IoUringMonitorRegistration>;
pub(super) type Storage = MonitorStorage<State>;

/// Persistent directional readiness registration. The ring retains its storage through the final original and cancellation completions; application handles independently control when the registration closes.
pub struct IoUringMonitorRegistration {
    group: Arc<DriverGroup>,
    slot: AtomicUsize,
    users: AtomicUsize,
    closed: AtomicBool,
    claimed: AtomicBool,
    pending: AtomicBool,
    readiness: AtomicU64,
    tcp: bool,
    read_started: AtomicBool,
    multishot: AtomicBool,
    status: Mutex<Status>,
}
#[derive(Default)]
struct Status {
    generation: u64,
    token: u64,
    controller: Option<Arc<Controller>>,
    retiring: bool,
    error: Option<io::Error>,
    waker: Option<Waker>,
}

fn wake(waker: Option<Waker>) {
    if let Some(waker) = waker {
        discard(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || waker.wake(),
        )));
    }
}

impl IoUringMonitorRegistration {
    fn clear_readiness(&self, observed: u64) {
        // Peer-shutdown readiness is permanent: clearing it after the last bytes could strand the next EOF read without another edge.
        if observed & SHUTDOWN != 0 {
            return;
        }
        // Preserve notifications published after the syscall took its readiness snapshot.
        discard(self.readiness.compare_exchange(
            observed,
            observed & !READY,
            Ordering::AcqRel,
            Ordering::Acquire,
        ));
    }

    // `try_update` is not available on the supported Rust 1.85 toolchain.
    #[allow(deprecated)]
    fn notify(&self, events: i32) {
        let waker = {
            let mut status = self.status.lock().unwrap();
            // The low bits cache readiness and permanent peer shutdown; the remaining bits distinguish notifications racing a syscall. Like readiness ticks in other reactors, this counter wraps.
            let terminal = if events & (libc::POLLHUP | libc::POLLRDHUP) as i32 != 0 {
                SHUTDOWN
            } else {
                0
            };
            discard(
                self.readiness
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                        Some(value.wrapping_add(4) | READY | terminal)
                    }),
            );
            status.waker.take()
        };
        wake(waker);
    }

    fn complete(&self, generation: u64, result: io::Result<()>) {
        let waker = {
            let mut status = self.status.lock().unwrap();
            assert_eq!(status.generation, generation);
            self.pending.store(false, Ordering::Release);
            status.controller = None;
            if !status.retiring {
                status.error = result.err();
            }
            status.retiring = false;
            status.waker.take()
        };
        wake(waker);
    }

    fn retire(&self, close: bool) {
        if close {
            self.closed.store(true, Ordering::Release);
        }
        let command = {
            let mut status = self.status.lock().unwrap();
            if status.retiring || !self.pending.load(Ordering::Acquire) {
                return;
            }
            status.retiring = true;
            (
                Arc::clone(status.controller.as_ref().unwrap()),
                status.token,
                status.generation,
            )
        };
        discard(command.0.send(Command::MonitorCancel(command.1, command.2)));
    }
}

impl MonitorState for IoUringMonitorRegistration {
    fn acquire(&self) {
        self.users.fetch_add(1, Ordering::Relaxed);
    }
    fn release(&self) {
        if self.users.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.retire(false);
        }
    }
}

struct Wait<'a> {
    storage: &'a State,
}
impl<'a> Wait<'a> {
    fn claim(storage: &'a State) -> io::Result<Self> {
        if storage
            .registration()
            .claimed
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "socket monitor already has a waiter",
            ));
        }
        Ok(Self { storage })
    }
}
impl Drop for Wait<'_> {
    fn drop(&mut self) {
        let previous = self
            .storage
            .registration()
            .status
            .lock()
            .unwrap()
            .waker
            .take();
        discard(previous);
        self.storage
            .registration()
            .claimed
            .store(false, Ordering::Release);
    }
}

impl IoUringSelector {
    fn check_monitor(&self, monitor: &State) -> io::Result<()> {
        self.check_open()?;
        if !Arc::ptr_eq(&self.group, &monitor.registration().group) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "socket monitor belongs to another selector",
            ));
        }
        if monitor.registration().closed.load(Ordering::Acquire) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        Ok(())
    }

    async fn monitor_wait<M: SocketMonitorState<NativeSocket, IoUringMonitorRegistration>>(
        &self,
        monitor: &MonitorStorage<M>,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        let state = monitor.as_ref();
        let registration = state.registration();
        cancellation.check()?;
        self.check_monitor(state)?;
        let _wait = Wait::claim(state)?;
        // Choose a route once for this wait. A suspended future can move without moving its active registration. The next wait may retire and rebind it after a worker change.
        let controller = self.route();
        let mut cancelled = pin!(cancellation.cancelled());
        poll_fn(|context| {
            if cancelled.as_mut().poll(context).is_ready() {
                return Poll::Ready(Err(core_scheduler::Cancelled.into()));
            }
            self.check_monitor(state)?;
            let replacement = context.waker().clone();
            let (previous, submission, retirement) = {
                let mut status = registration.status.lock().unwrap();
                if !status.retiring {
                    if let Some(error) = status.error.take() {
                        return Poll::Ready(Err(error.into()));
                    }
                    if registration.readiness.load(Ordering::Acquire) & READY != 0 {
                        return Poll::Ready(Ok(()));
                    }
                }
                let mut submission = None;
                let mut retirement = None;
                if registration.pending.load(Ordering::Acquire) {
                    if !status.retiring
                        && !Arc::ptr_eq(status.controller.as_ref().unwrap(), &controller)
                    {
                        status.retiring = true;
                        retirement = Some((
                            Arc::clone(status.controller.as_ref().unwrap()),
                            status.token,
                            status.generation,
                        ));
                    }
                } else {
                    status.generation = Self::next_identifier(&controller)?;
                    status.token = state as *const State as u64 | MONITOR_TAG;
                    status.controller = Some(Arc::clone(&controller));
                    status.retiring = false;
                    registration.pending.store(true, Ordering::Release);
                    submission = Some(Submission {
                        storage: monitor.retained_project(AsRef::as_ref),
                        generation: status.generation,
                        finished: false,
                    });
                }
                (status.waker.replace(replacement), submission, retirement)
            };
            discard(previous);
            if let Some((controller, token, generation)) = retirement {
                discard(controller.send(Command::MonitorCancel(token, generation)));
            }
            // Rejection drops the submission, publishes BrokenPipe and releases backend retention.
            if let Some(submission) = submission {
                discard(controller.send(Command::MonitorSubmit(submission)));
            }
            Poll::Pending
        })
        .await
    }

    async fn monitor_close<M: SocketMonitorState<NativeSocket, IoUringMonitorRegistration>>(
        &self,
        monitor: &MonitorStorage<M>,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        let state = monitor.as_ref();
        let registration = state.registration();
        cancellation.check()?;
        if !Arc::ptr_eq(&self.group, &registration.group) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "socket monitor belongs to another selector",
            )
            .into());
        }
        let _wait = Wait::claim(state)?;
        registration.retire(true);
        let mut cancelled = pin!(cancellation.cancelled());
        poll_fn(|context| {
            if cancelled.as_mut().poll(context).is_ready() {
                return Poll::Ready(Err(core_scheduler::Cancelled.into()));
            }
            let replacement = context.waker().clone();
            let previous = {
                let mut status = registration.status.lock().unwrap();
                if !registration.pending.load(Ordering::Acquire) {
                    return Poll::Ready(Ok(()));
                }
                status.waker.replace(replacement)
            };
            discard(previous);
            Poll::Pending
        })
        .await
    }

    async fn monitor_io<T, M: SocketMonitorState<NativeSocket, IoUringMonitorRegistration>>(
        &self,
        monitor: &MonitorStorage<M>,
        interest: Interest,
        cancellation: &Cancellation,
        operation: impl FnMut(&NativeSocket) -> io::Result<T> + Send,
    ) -> OperationResult<T> {
        self.monitor_io_with(monitor, interest, cancellation, false, operation, |_| false)
            .await
    }

    async fn monitor_io_with<T, M: SocketMonitorState<NativeSocket, IoUringMonitorRegistration>>(
        &self,
        monitor: &MonitorStorage<M>,
        interest: Interest,
        cancellation: &Cancellation,
        ready_first: bool,
        mut operation: impl FnMut(&NativeSocket) -> io::Result<T> + Send,
        drained: impl Fn(&T) -> bool + Send,
    ) -> OperationResult<T> {
        let state = monitor.as_ref();
        let registration = state.registration();
        cancellation.check()?;
        self.check_monitor(state)?;
        state.check_interest(interest)?;
        loop {
            // Only an established stream's nonempty, consuming receive may skip the speculative syscall. Other operations retain immediate OS results.
            if ready_first
                && registration.read_started.load(Ordering::Acquire)
                && registration.readiness.load(Ordering::Acquire) & READY == 0
            {
                self.monitor_wait(monitor, cancellation).await?;
            }
            let readiness = registration.readiness.load(Ordering::Acquire);
            match operation(state.socket()) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    registration.clear_readiness(readiness);
                    self.monitor_wait(monitor, cancellation).await?
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                    cancellation.check()?;
                    self.check_monitor(state)?;
                }
                Ok(value) => {
                    if drained(&value) {
                        registration.read_started.store(true, Ordering::Release);
                        registration.clear_readiness(readiness);
                    }
                    return Ok(value);
                }
                result => return result.map_err(Into::into),
            }
        }
    }
}

impl SocketMonitoring for IoUringSelector {
    type MonitorRegistration = IoUringMonitorRegistration;
    fn socket_monitor_state(&self, socket: &NativeSocket, interest: Interest) -> io::Result<State> {
        self.check_open()?;
        let (socket, ()) = self
            .fallback
            .socket_monitor_state(socket, interest)
            .expect("readiness monitor construction only retains the socket")
            .into_parts();
        Ok(SocketMonitor::new(
            socket,
            interest,
            IoUringMonitorRegistration {
                group: Arc::clone(&self.group),
                slot: AtomicUsize::new(NO_SLOT),
                users: AtomicUsize::new(0),
                closed: AtomicBool::new(false),
                readiness: AtomicU64::new(0),
                tcp: false,
                read_started: AtomicBool::new(false),
                multishot: AtomicBool::new(true),
                claimed: AtomicBool::new(false),
                pending: AtomicBool::new(false),
                status: Mutex::new(Status::default()),
            },
        ))
    }
    fn socket_stream_monitor_state(
        &self,
        socket: &NativeSocket,
        interest: Interest,
    ) -> io::Result<SocketStreamMonitor<NativeSocket, IoUringMonitorRegistration>> {
        let monitor = self.socket_monitor_state(socket, interest)?;
        // Cache TCP recognition at setup, never on the payload path. Other transports and failed queries keep the conservative syscall-first behavior.
        let native = socket::retain_socket(socket);
        let tcp = syscall("monitor-socket-type", || native.get_ref().r#type())
            .is_ok_and(|kind| kind == socket2::Type::STREAM)
            && syscall("monitor-socket-protocol", || native.get_ref().protocol())
                .is_ok_and(|protocol| protocol == Some(socket2::Protocol::TCP));
        let (socket, mut registration) = monitor.into_parts();
        registration.tcp = tcp;
        Ok(SocketStreamMonitor::new(SocketMonitor::new(
            socket,
            interest,
            registration,
        )))
    }
    async fn socket_monitor_wait<
        M: SocketMonitorState<NativeSocket, IoUringMonitorRegistration>,
    >(
        &self,
        monitor: &mut MonitorStorage<M>,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        self.monitor_wait(monitor, cancellation).await
    }
    async fn socket_monitor_close<
        M: SocketMonitorState<NativeSocket, IoUringMonitorRegistration>,
    >(
        &self,
        monitor: &mut MonitorStorage<M>,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        self.monitor_close(monitor, cancellation).await
    }
    async fn socket_monitor_recv_poll<
        M: SocketMonitorState<NativeSocket, IoUringMonitorRegistration>,
    >(
        &self,
        monitor: &mut MonitorStorage<M>,
        buffer: &mut [u8],
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        let length = buffer.len();
        let ready_first =
            M::STREAM && monitor.as_ref().registration().tcp && length != 0 && !flags.peek;
        self.monitor_io_with(
            monitor,
            Interest::Readable,
            cancellation,
            ready_first,
            |socket| syscall("monitor-recv", || socket.try_recv(buffer, flags)),
            |count| ready_first && 0 < *count && *count < length,
        )
        .await
    }
    async fn socket_monitor_send_poll<
        M: SocketMonitorState<NativeSocket, IoUringMonitorRegistration>,
    >(
        &self,
        monitor: &mut MonitorStorage<M>,
        buffer: &[u8],
        flags: SocketSendFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.monitor_io(monitor, Interest::Writable, cancellation, |socket| {
            socket.try_send(buffer, flags)
        })
        .await
    }
    async fn socket_monitor_recvmsg_poll<
        M: SocketMonitorState<NativeSocket, IoUringMonitorRegistration>,
    >(
        &self,
        monitor: &mut MonitorStorage<M>,
        buffers: &mut [std::io::IoSliceMut<'_>],
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<SocketRecvResult> {
        self.monitor_io(monitor, Interest::Readable, cancellation, |socket| {
            socket.try_recvmsg(buffers, flags)
        })
        .await
    }
    async fn socket_monitor_sendmsg_poll<
        M: SocketMonitorState<NativeSocket, IoUringMonitorRegistration>,
    >(
        &self,
        monitor: &mut MonitorStorage<M>,
        buffers: &[std::io::IoSlice<'_>],
        address: Option<SocketAddress>,
        flags: SocketSendFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.monitor_io(monitor, Interest::Writable, cancellation, |socket| {
            socket.try_sendmsg(buffers, address, flags)
        })
        .await
    }
}

pub(crate) struct Submission {
    storage: Storage,
    generation: u64,
    finished: bool,
}
impl Submission {
    fn token(&self) -> u64 {
        self.storage.as_ptr() as u64 | MONITOR_TAG
    }
    pub(super) fn finish(mut self, result: io::Result<()>) {
        self.finished = true;
        self.storage
            .registration()
            .complete(self.generation, result);
    }
    pub(super) fn cancelled(&self) -> bool {
        self.storage.registration().status.lock().unwrap().retiring
    }
}

impl Drop for Submission {
    fn drop(&mut self) {
        if !self.finished {
            self.storage
                .registration()
                .complete(self.generation, Err(io::ErrorKind::BrokenPipe.into()));
        }
    }
}

struct Entry {
    submission: Submission,
    original: Option<io::Result<()>>,
    cancelling: bool,
    cancel_done: bool,
}

/// The worker owns every pointer published in an SQE. Slots are reused only after original and cancellation CQEs have both retired; completion dispatch uses the record pointer and its slot directly, never a hash lookup.
pub(super) struct Monitors {
    slots: Vec<Option<Entry>>,
    free: Vec<usize>,
    len: usize,
}
impl Monitors {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            slots: (0..capacity).map(|_| None).collect(),
            free: (0..capacity).rev().collect(),
            len: 0,
        }
    }
    pub(super) fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub(super) fn insert(&mut self, submission: Submission) -> squeue::Entry {
        // Persistent registrations must not consume transient operation permits: an idle connection must not prevent another connection from registering. Growth is confined to first registration when reserved backend capacity is exhausted.
        let slot = self.free.pop().unwrap_or_else(|| {
            self.slots.push(None);
            self.slots.len() - 1
        });
        let events = match submission.storage.interest() {
            Interest::Readable => libc::POLLIN | libc::POLLRDHUP,
            Interest::Writable => libc::POLLOUT,
        };
        let entry = opcode::PollAdd::new(
            types::Fd(
                super::socket::retain_socket(submission.storage.socket())
                    .get_ref()
                    .as_raw_fd(),
            ),
            events as u32,
        )
        .multi(
            submission
                .storage
                .registration()
                .multishot
                .load(Ordering::Relaxed),
        )
        .build()
        .user_data(submission.token());
        submission
            .storage
            .registration()
            .slot
            .store(slot, Ordering::Relaxed);
        self.slots[slot] = Some(Entry {
            submission,
            original: None,
            cancelling: false,
            cancel_done: false,
        });
        self.len += 1;
        entry
    }
    // SAFETY: Only original or cancellation CQEs for our live submissions call this function. The slot owns the published address until both kinds of CQE have retired. The kernel treats user_data as opaque and does not access the pointed-to memory.
    fn slot(&self, token: u64) -> usize {
        let pointer = (token & !MONITOR_TAG) as *const State;
        let slot = unsafe { &*pointer }
            .registration()
            .slot
            .load(Ordering::Relaxed);
        assert_eq!(
            self.slots[slot]
                .as_ref()
                .unwrap()
                .submission
                .storage
                .as_ptr(),
            pointer
        );
        slot
    }
    pub(super) fn complete(&mut self, token: u64, result: i32, more: bool, cancelled: bool) {
        let slot = self.slot(token);
        let entry = self.slots[slot].as_mut().unwrap();
        if cancelled {
            entry.cancel_done = true;
        } else {
            if more {
                assert!(
                    result >= 0,
                    "multishot poll errors carry a final CQE without MORE"
                );
                entry.submission.storage.registration().notify(result);
                return;
            }
            let completion_events = result;
            let result = if result < 0 {
                Err(io::Error::from_raw_os_error(-result))
            } else {
                Ok(())
            };
            let fallback = matches!(&result, Err(error) if error.raw_os_error() == Some(libc::EINVAL))
                && entry
                    .submission
                    .storage
                    .registration()
                    .multishot
                    .swap(false, Ordering::Relaxed);
            if fallback {
                entry.original = Some(Ok(()));
            } else {
                if result.is_ok() {
                    entry
                        .submission
                        .storage
                        .registration()
                        .notify(completion_events);
                }
                entry.original = Some(result);
            }
        }
        if entry.original.is_some() && (!entry.cancelling || entry.cancel_done) {
            let mut entry = self.slots[slot].take().unwrap();
            self.free.push(slot);
            self.len -= 1;
            entry
                .submission
                .storage
                .registration()
                .slot
                .store(NO_SLOT, Ordering::Relaxed);
            entry.submission.finish(entry.original.take().unwrap());
        }
    }
    pub(super) fn cancel(&mut self, token: u64, generation: u64) -> Option<u64> {
        // Cancellation is cold. Compare opaque addresses against live owners instead of dereferencing a possibly stale pointer from a delayed command. Generations are unique per controller, including across address reuse.
        let entry = self.slots.iter_mut().flatten().find(|entry| {
            entry.submission.token() == token && entry.submission.generation == generation
        })?;
        if entry.cancelling {
            return None;
        }
        entry.cancelling = true;
        Some(token)
    }
    pub(super) fn close(&mut self, cancellations: &mut VecDeque<u64>) {
        for entry in self.slots.iter_mut().flatten() {
            if !entry.cancelling {
                entry.cancelling = true;
                cancellations.push_back(entry.submission.token());
            }
        }
    }
}
impl Drop for Monitors {
    fn drop(&mut self) {
        for entry in self.slots.iter_mut().filter_map(Option::take) {
            entry
                .submission
                .storage
                .registration()
                .slot
                .store(NO_SLOT, Ordering::Relaxed);
            entry
                .submission
                .finish(Err(io::ErrorKind::BrokenPipe.into()));
        }
    }
}

#[cfg(test)]
mod tests;
