// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::{OperationError, SocketOperation, SocketRecvResult};
use futures_lite::future::{block_on, poll_once};
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Socket {
    state: Arc<SocketState>,
}
struct SocketState {
    incoming: Mutex<VecDeque<io::Result<&'static [u8]>>>,
    changed: Event,
    dropped: AtomicUsize,
}
impl Drop for Socket {
    fn drop(&mut self) {
        self.state.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct Controlled {
    calls: AtomicUsize,
    incompatible: bool,
}
impl SocketOperation for Controlled {
    type Socket = Socket;
}
impl SocketRecvMsgPoll for Controlled {
    async fn socket_recvmsg_poll(
        &self,
        socket: &Socket,
        buffers: &mut [IoSliceMut<'_>],
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<SocketRecvResult> {
        cancellation.check()?;
        assert!(!flags.peek);
        self.calls.fetch_add(1, Ordering::Relaxed);
        let mut cancelled = pin!(cancellation.cancelled());
        loop {
            event_listener::listener!(socket.state.changed => listener);
            let incoming = socket.state.incoming.lock().unwrap().pop_front();
            if let Some(incoming) = incoming {
                let bytes = incoming?;
                let length = bytes.len().min(buffers[0].len());
                buffers[0][..length].copy_from_slice(&bytes[..length]);
                return Ok(SocketRecvResult {
                    bytes: length,
                    address: Some("127.0.0.1:4567".parse().unwrap()),
                    truncated: length < bytes.len(),
                    control_truncated: false,
                });
            }
            let mut listener = pin!(listener);
            let ready = poll_fn(|context| {
                if listener.as_mut().poll(context).is_ready() {
                    Poll::Ready(true)
                } else {
                    cancelled.as_mut().poll(context).map(|_| false)
                }
            })
            .await;
            if !ready {
                return Err(Cancelled.into());
            }
        }
    }
}

type Receiver = PollingRecvMsg<Socket, fn(&Controlled, &Socket) -> io::Result<()>>;

fn compatible(scheduler: &Controlled, _socket: &Socket) -> io::Result<()> {
    if scheduler.incompatible {
        Err(io::ErrorKind::InvalidInput.into())
    } else {
        Ok(())
    }
}

fn receiver(kind: SocketType, buffers: u16) -> (Receiver, Arc<SocketState>) {
    let changed = Event::new();
    changed.notify(0);
    let state = Arc::new(SocketState {
        incoming: Mutex::new(VecDeque::new()),
        changed,
        dropped: AtomicUsize::new(0),
    });
    let receiver = PollingRecvMsg::new(
        Socket {
            state: Arc::clone(&state),
        },
        kind,
        RecvMsgMultishotOptions {
            buffers,
            buffer_size: 4,
        },
        SocketRecvFlags::default(),
        compatible as fn(&Controlled, &Socket) -> io::Result<()>,
    )
    .unwrap();
    (receiver, state)
}
fn send(state: &SocketState, incoming: io::Result<&'static [u8]>) {
    state.incoming.lock().unwrap().push_back(incoming);
    state.changed.notify(1);
}

#[test]
fn completed_prefix_metadata_and_pool_reuse_do_not_allocate() {
    let scheduler = Controlled::default();
    let (mut receiving, state) = receiver(SocketType::Datagram, 2);
    state
        .incoming
        .lock()
        .unwrap()
        .extend((0..20).map(|_| Ok(b"abcdef".as_slice())));
    let never = Cancellation::never();
    let allocations = allocation_counter::measure(|| {
        let mut original = None;
        for _ in 0..20 {
            let mut next = pin!(receiving.recv(&scheduler, &never));
            let Poll::Ready(Ok(Some(message))) = next
                .as_mut()
                .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
            else {
                panic!("queued receive must complete immediately");
            };
            assert_eq!(message.buffer.as_ref(), b"abcd");
            assert_eq!(message.result.bytes, 4);
            assert_eq!(
                message.result.address,
                Some("127.0.0.1:4567".parse().unwrap())
            );
            assert!(message.result.truncated);
            assert!(!message.result.control_truncated);
            let pointer = message.buffer.as_ref().as_ptr();
            assert_eq!(*original.get_or_insert(pointer), pointer);
            drop(message);
        }
    });
    assert_eq!(allocations.count_total, 0, "{allocations:?}");
    assert_eq!(scheduler.calls.load(Ordering::Relaxed), 20);
    block_on(receiving.close(&scheduler)).unwrap();
    assert_eq!(state.dropped.load(Ordering::Relaxed), 1);
}

#[test]
fn exhaustion_waits_for_a_returned_lease_and_can_move_between_schedulers_and_threads() {
    let first_scheduler = Controlled::default();
    let second_scheduler = Controlled::default();
    let (mut receiving, state) = receiver(SocketType::Stream, 1);
    send(&state, Ok(b"one"));
    let first = block_on(receiving.recv(&first_scheduler, &Cancellation::never()))
        .unwrap()
        .unwrap();
    send(&state, Ok(b"two"));
    let never = Cancellation::never();
    let mut next = Box::pin(receiving.recv(&second_scheduler, &never));
    assert!(block_on(poll_once(&mut next)).is_none());
    assert_eq!(second_scheduler.calls.load(Ordering::Relaxed), 0);
    std::thread::spawn(move || drop(first)).join().unwrap();
    let second = block_on(next).unwrap().unwrap();
    assert_eq!(second.buffer.as_ref(), b"two");
    assert_eq!(first_scheduler.calls.load(Ordering::Relaxed), 1);
    assert_eq!(second_scheduler.calls.load(Ordering::Relaxed), 1);
    block_on(receiving.close(&second_scheduler)).unwrap();
    drop(receiving);
    assert_eq!(second.buffer.as_ref(), b"two");
}

#[test]
fn pending_cancellation_closes_both_pool_and_socket_waits_but_precancellation_preserves() {
    for exhausted in [false, true] {
        let scheduler = Controlled::default();
        let (mut receiving, state) = receiver(SocketType::Stream, 1);
        let pre = Cancellation::new();
        pre.cancel();
        assert!(matches!(
            block_on(receiving.recv(&scheduler, &pre)),
            Err(OperationError::Cancelled(_))
        ));
        assert_eq!(scheduler.calls.load(Ordering::Relaxed), 0);
        let held = if exhausted {
            send(&state, Ok(b"held"));
            Some(
                block_on(receiving.recv(&scheduler, &Cancellation::never()))
                    .unwrap()
                    .unwrap(),
            )
        } else {
            None
        };
        let token = Cancellation::new();
        let mut next = Box::pin(receiving.recv(&scheduler, &token));
        assert!(block_on(poll_once(&mut next)).is_none());
        token.cancel();
        assert!(matches!(block_on(next), Err(OperationError::Cancelled(_))));
        assert_eq!(state.dropped.load(Ordering::Relaxed), 1);
        assert!(
            block_on(receiving.recv(&scheduler, &Cancellation::never()))
                .unwrap()
                .is_none()
        );
        if let Some(held) = held {
            assert_eq!(held.buffer.as_ref(), b"held");
        }
    }
}

#[test]
fn dropping_pending_receives_returns_capacity_without_closing_or_independent_io() {
    let scheduler = Controlled::default();
    let (mut receiving, state) = receiver(SocketType::Stream, 1);
    let never = Cancellation::never();
    let mut pending = Box::pin(receiving.recv(&scheduler, &never));
    assert!(block_on(poll_once(&mut pending)).is_none());
    drop(pending);
    assert_eq!(state.dropped.load(Ordering::Relaxed), 0);
    send(&state, Ok(b"next"));
    let held = block_on(receiving.recv(&scheduler, &never))
        .unwrap()
        .unwrap();
    let mut waiting = Box::pin(receiving.recv(&scheduler, &never));
    assert!(block_on(poll_once(&mut waiting)).is_none());
    drop(waiting);
    drop(held);
    send(&state, Ok(b"last"));
    assert_eq!(
        block_on(receiving.recv(&scheduler, &never))
            .unwrap()
            .unwrap()
            .buffer
            .as_ref(),
        b"last"
    );
    let lazy = receiving.recv(&scheduler, &never);
    drop(lazy);
    assert_eq!(scheduler.calls.load(Ordering::Relaxed), 3);
}

#[test]
fn stream_eof_is_terminal_while_an_empty_datagram_remains_a_message() {
    let scheduler = Controlled::default();
    for kind in [SocketType::Stream, SocketType::Datagram] {
        let (mut receiving, state) = receiver(kind, 1);
        send(&state, Ok(b""));
        let result = block_on(receiving.recv(&scheduler, &Cancellation::never())).unwrap();
        if kind == SocketType::Stream {
            assert!(result.is_none());
            assert_eq!(state.dropped.load(Ordering::Relaxed), 1);
            assert!(
                block_on(receiving.recv(&scheduler, &Cancellation::never()))
                    .unwrap()
                    .is_none()
            );
        } else {
            assert!(result.unwrap().buffer.as_ref().is_empty());
            send(&state, Ok(b"more"));
            assert_eq!(
                block_on(receiving.recv(&scheduler, &Cancellation::never()))
                    .unwrap()
                    .unwrap()
                    .buffer
                    .as_ref(),
                b"more"
            );
        }
    }
}

#[test]
fn errors_retire_once_and_explicit_close_is_idempotent() {
    let scheduler = Controlled::default();
    let (mut receiving, state) = receiver(SocketType::Stream, 1);
    send(&state, Err(io::ErrorKind::ConnectionReset.into()));
    assert!(
        matches!(block_on(receiving.recv(&scheduler, &Cancellation::never())), Err(OperationError::Io(error)) if error.kind() == io::ErrorKind::ConnectionReset)
    );
    assert!(
        block_on(receiving.recv(&scheduler, &Cancellation::never()))
            .unwrap()
            .is_none()
    );
    for _ in 0..2 {
        block_on(receiving.close(&scheduler)).unwrap();
    }
    assert_eq!(state.dropped.load(Ordering::Relaxed), 1);
    let (mut receiving, state) = receiver(SocketType::Datagram, 1);
    send(&state, Ok(b"keep"));
    let held = block_on(receiving.recv(&scheduler, &Cancellation::never()))
        .unwrap()
        .unwrap();
    block_on(receiving.close(&scheduler)).unwrap();
    drop(receiving);
    assert_eq!(held.buffer.as_ref(), b"keep");
}

#[test]
fn invalid_options_and_peeking_do_not_create_a_resource() {
    let (_receiving, state) = receiver(SocketType::Stream, 1);
    for (buffers, buffer_size) in [(0, 1), (65535, 1), (1, 0), (32768, usize::MAX)] {
        assert!(
            matches!(PollingRecvMsg::new(Socket { state: Arc::clone(&state) }, SocketType::Stream, RecvMsgMultishotOptions { buffers, buffer_size }, SocketRecvFlags::default(), compatible), Err(error) if error.kind() == io::ErrorKind::InvalidInput)
        );
    }
    assert!(
        matches!(PollingRecvMsg::new(Socket { state: Arc::clone(&state) }, SocketType::Stream, RecvMsgMultishotOptions::default(), SocketRecvFlags { peek: true }, compatible), Err(error) if error.kind() == io::ErrorKind::InvalidInput)
    );
    assert_eq!(state.dropped.load(Ordering::Relaxed), 5);
}

#[test]
fn incompatible_schedulers_do_not_consume_capacity_data_or_close_the_receiver() {
    let scheduler = Controlled::default();
    let incompatible = Controlled {
        incompatible: true,
        ..Default::default()
    };
    let (mut receiving, state) = receiver(SocketType::Datagram, 1);
    send(&state, Ok(b"keep"));
    assert!(
        matches!(block_on(receiving.recv(&incompatible, &Cancellation::never())), Err(OperationError::Io(error)) if error.kind() == io::ErrorKind::InvalidInput)
    );
    assert_eq!(
        block_on(receiving.close(&incompatible)).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(state.dropped.load(Ordering::Relaxed), 0);
    let held = block_on(receiving.recv(&scheduler, &Cancellation::never()))
        .unwrap()
        .unwrap();
    assert_eq!(held.buffer.as_ref(), b"keep");
    // Validation also precedes a full-pool wait; rejection must be immediate.
    assert!(
        matches!(block_on(receiving.recv(&incompatible, &Cancellation::never())), Err(OperationError::Io(error)) if error.kind() == io::ErrorKind::InvalidInput)
    );
    block_on(receiving.close(&scheduler)).unwrap();
    assert_eq!(state.dropped.load(Ordering::Relaxed), 1);
    assert!(
        block_on(receiving.recv(&incompatible, &Cancellation::never()))
            .unwrap()
            .is_none()
    );
    block_on(receiving.close(&incompatible)).unwrap();
}
