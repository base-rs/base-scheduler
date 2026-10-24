// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::ring_binding::RingBinding;
use super::*;
use crate::Interest;
use std::sync::Mutex;
use std::task::Waker;

pub(super) const MONITOR_TAG: u64 = 1;
const READY: u64 = 1;
const NO_SLOT: usize = usize::MAX;
pub(crate) type State = SocketMonitor;
pub(crate) type Storage = MonitorStorage;

/// Persistent directional readiness registration. The ring retains its storage through the final original and cancellation completions; application handles independently control when the registration closes.
pub(crate) struct IoUringMonitorRegistration {
    group: Arc<DriverGroup>,
    slot: AtomicUsize,
    users: AtomicUsize,
    closed: AtomicBool,
    claimed: AtomicBool,
    readiness: AtomicU64,
    multishot: AtomicBool,
    status: Arc<Mutex<Status>>,
}
#[derive(Default)]
struct Status {
    pending: bool,
    binding: Option<RingBinding<PollRegistration>>,
    retiring: bool,
    error: Option<io::Error>,
    waker: Option<Waker>,
    retirement_waker: Option<Waker>,
}

struct PollRegistration {
    generation: u64,
    token: u64,
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
    fn notify(&self) {
        let waker = {
            let mut status = self.status.lock().unwrap();
            // The low bit caches readiness; the remaining bits distinguish notifications racing a syscall. Like readiness ticks in other reactors, this counter wraps.
            discard(
                self.readiness
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                        Some(value.wrapping_add(2) | READY)
                    }),
            );
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
            if status.retiring || !status.pending {
                return;
            }
            status.retiring = true;
            let binding = status.binding.as_ref().unwrap();
            (
                Arc::clone(&binding.controller),
                binding.state.token,
                binding.state.generation,
            )
        };
        discard(command.0.send(Command::MonitorCancel(command.1, command.2)));
    }
}

impl IoUringMonitorRegistration {
    pub(crate) fn acquire(&self) {
        self.users.fetch_add(1, Ordering::Relaxed);
    }
    pub(crate) fn release(&self) {
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

    async fn monitor_wait(
        &self,
        monitor: &Storage,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        let state = monitor.as_ref();
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
            self.poll_monitor_wait(monitor, &controller, context)
        })
        .await
    }

    // The caller holds the waiter claim and checks cancellation and monitor compatibility before attempting readiness.
    fn poll_monitor_wait(
        &self,
        monitor: &Storage,
        controller: &Arc<Controller>,
        context: &mut std::task::Context<'_>,
    ) -> Poll<OperationResult<()>> {
        let state = monitor.as_ref();
        let registration = state.registration();
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
            if status.pending {
                if !status.retiring && !status.binding.as_ref().unwrap().matches(controller) {
                    status.retiring = true;
                    let binding = status.binding.as_ref().unwrap();
                    retirement = Some((
                        Arc::clone(&binding.controller),
                        binding.state.token,
                        binding.state.generation,
                    ));
                }
            } else {
                let generation = controller.next_identifier()?;
                status.binding = Some(RingBinding::new(
                    controller,
                    PollRegistration {
                        generation,
                        token: state as *const State as u64 | MONITOR_TAG,
                    },
                ));
                status.retiring = false;
                status.pending = true;
                submission = Some(Submission::new(
                    monitor.retained(),
                    Arc::clone(&registration.status),
                    generation,
                ));
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
    }

    async fn monitor_close(&self, monitor: &Storage) -> io::Result<()> {
        let state = monitor.as_ref();
        let registration = state.registration();
        if !Arc::ptr_eq(&self.group, &registration.group) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "socket monitor belongs to another selector",
            ));
        }
        registration.retire(true);
        poll_fn(|context| {
            let replacement = context.waker().clone();
            let previous = {
                let mut status = registration.status.lock().unwrap();
                if !status.pending {
                    return Poll::Ready(Ok(()));
                }
                status.retirement_waker.replace(replacement)
            };
            discard(previous);
            Poll::Pending
        })
        .await
    }

    async fn monitor_io<T>(
        &self,
        monitor: &Storage,
        interest: Interest,
        cancellation: &Cancellation,
        mut operation: impl FnMut(&RegisteredSocket) -> io::Result<T> + Send,
    ) -> OperationResult<T> {
        let state = monitor.as_ref();
        let registration = state.registration();
        let mut wait = None;
        let mut cancelled = pin!(cancellation.cancelled());
        poll_fn(|context| {
            loop {
                cancellation.check()?;
                self.check_monitor(state)?;
                state.check_interest(interest)?;
                let readiness = registration.readiness.load(Ordering::Acquire);
                match operation(state.socket()) {
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        registration.clear_readiness(readiness);
                        // Keep a single waiter and its original route until this operation completes. A wake retries the syscall before touching readiness bookkeeping.
                        if wait.is_none() {
                            wait = Some((Wait::claim(state)?, self.route()));
                        }
                        if cancelled.as_mut().poll(context).is_ready() {
                            return Poll::Ready(Err(core_scheduler::Cancelled.into()));
                        }
                        let (_, controller) = wait.as_ref().unwrap();
                        std::task::ready!(self.poll_monitor_wait(monitor, controller, context))?;
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    result => return Poll::Ready(result.map_err(Into::into)),
                }
            }
        })
        .await
    }
}

impl IoUringSelector {
    pub(crate) fn socket_type(&self, socket: &impl SocketSource) -> io::Result<SocketType> {
        self.check_open()?;
        socket::retain_socket(socket)
            .get_ref()
            .r#type()
            .map(|kind| {
                if kind == socket2::Type::STREAM {
                    SocketType::Stream
                } else {
                    SocketType::Other(kind.into())
                }
            })
    }
    // Factories validate admission and socket kind before creating this inert record. Registration starts only when an operation needs readiness.
    pub(crate) fn socket_monitor(&self, socket: &impl SocketSource, interest: Interest) -> Storage {
        MonitorStorage::new(SocketMonitor::new(
            socket::retain_socket(socket),
            interest,
            IoUringMonitorRegistration {
                group: Arc::clone(&self.group),
                slot: AtomicUsize::new(NO_SLOT),
                users: AtomicUsize::new(0),
                closed: AtomicBool::new(false),
                readiness: AtomicU64::new(0),
                multishot: AtomicBool::new(true),
                claimed: AtomicBool::new(false),
                status: Arc::new(Mutex::new(Status::default())),
            },
        ))
    }
    pub(crate) async fn socket_monitor_wait(
        &self,
        monitor: &mut Storage,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        self.monitor_wait(monitor, cancellation).await
    }
    pub(crate) async fn socket_monitor_close(&self, monitor: &mut Storage) -> io::Result<()> {
        self.monitor_close(monitor).await
    }
    pub(crate) async fn socket_monitor_recv_poll(
        &self,
        monitor: &mut Storage,
        buffer: &mut [u8],
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.monitor_io(monitor, Interest::Readable, cancellation, |socket| {
            syscall("monitor-recv", || {
                crate::retained_socket::recv(socket, buffer, flags)
            })
        })
        .await
    }
    pub(crate) async fn socket_monitor_send_poll(
        &self,
        monitor: &mut Storage,
        buffer: &[u8],
        flags: SocketSendFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.monitor_io(monitor, Interest::Writable, cancellation, |socket| {
            crate::retained_socket::send(socket, buffer, flags)
        })
        .await
    }
    pub(crate) async fn socket_monitor_recvmsg_poll(
        &self,
        monitor: &mut Storage,
        buffers: &mut [std::io::IoSliceMut<'_>],
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<SocketRecvResult> {
        self.monitor_io(monitor, Interest::Readable, cancellation, |socket| {
            crate::retained_socket::recvmsg(socket, buffers, flags)
        })
        .await
    }
    pub(crate) async fn socket_monitor_sendmsg_poll(
        &self,
        monitor: &mut Storage,
        buffers: &[std::io::IoSlice<'_>],
        address: Option<SocketAddress>,
        flags: SocketSendFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.monitor_io(monitor, Interest::Writable, cancellation, |socket| {
            crate::retained_socket::sendmsg(socket, buffers, address, flags)
        })
        .await
    }
}

pub(crate) struct Submission {
    // Fields drop in declaration order: release the resource owner before retirement publishes completion, even when a submission is rejected or abandoned.
    storage: Storage,
    retirement: Retirement,
}
impl Submission {
    fn new(storage: Storage, status: Arc<Mutex<Status>>, generation: u64) -> Self {
        Self {
            storage,
            retirement: Retirement {
                status,
                generation,
                error: Some(io::ErrorKind::BrokenPipe.into()),
            },
        }
    }
    fn token(&self) -> u64 {
        self.storage.as_ptr() as u64 | MONITOR_TAG
    }
    pub(super) fn finish(mut self, result: io::Result<()>) {
        self.retirement.error = result.err();
    }
    pub(super) fn cancelled(&self) -> bool {
        self.retirement.status.lock().unwrap().retiring
    }
}

// Independent of the monitor's resource-retaining storage, so completion cannot race its destruction or retain a socket that the caller may immediately close.
struct Retirement {
    status: Arc<Mutex<Status>>,
    generation: u64,
    error: Option<io::Error>,
}
impl Drop for Retirement {
    fn drop(&mut self) {
        let (waker, retirement_waker) = {
            let mut status = self.status.lock().unwrap();
            assert_eq!(
                status.binding.as_ref().unwrap().state.generation,
                self.generation
            );
            status.pending = false;
            status.binding = None;
            if !status.retiring {
                status.error = self.error.take();
            }
            status.retiring = false;
            (status.waker.take(), status.retirement_waker.take())
        };
        wake(waker);
        wake(retirement_waker);
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
                Arc::clone(submission.storage.socket())
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
                entry.submission.storage.registration().notify();
                return;
            }
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
                    entry.submission.storage.registration().notify();
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
            entry.submission.token() == token
                && entry.submission.retirement.generation == generation
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
