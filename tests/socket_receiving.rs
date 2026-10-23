// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::*;
#[path = "support/buffer_pool.rs"]
mod pool_support;
use futures_lite::future::{block_on, poll_once};
use pool_support::TestPool;
use std::{
    future::pending,
    io::{self, IoSlice, IoSliceMut},
    pin::pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

struct Sockets;
impl SocketOperation for Sockets {
    type Socket = Arc<AtomicUsize>;
}
impl SocketRecvPoll for Sockets {
    async fn socket_recv_poll(
        &self,
        socket: &Self::Socket,
        buffer: &mut [u8],
        _: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        cancellation.check()?;
        match socket.load(Ordering::Relaxed) {
            0 => Ok(0),
            1 => {
                buffer.fill(42);
                Ok(buffer.len())
            }
            2 => Err(io::Error::from(io::ErrorKind::InvalidInput).into()),
            3 => Err(io::Error::from(io::ErrorKind::WouldBlock).into()),
            4 => Err(io::Error::from(io::ErrorKind::BrokenPipe).into()),
            5 => {
                cancellation.cancel();
                Err(Cancelled.into())
            }
            _ => pending().await,
        }
    }
}
impl SocketSendPoll for Sockets {
    async fn socket_send_poll(
        &self,
        _: &Self::Socket,
        buffer: &[u8],
        _: SocketSendFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        cancellation.check()?;
        Ok(buffer.len())
    }
}
impl SocketRecvMsgPoll for Sockets {
    async fn socket_recvmsg_poll(
        &self,
        socket: &Self::Socket,
        buffers: &mut [IoSliceMut<'_>],
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<SocketRecvResult> {
        let bytes = self
            .socket_recv_poll(socket, &mut buffers[0], flags, cancellation)
            .await?;
        Ok(SocketRecvResult {
            bytes,
            address: None,
            truncated: false,
            control_truncated: false,
        })
    }
}
impl SocketSendMsgPoll for Sockets {
    async fn socket_sendmsg_poll(
        &self,
        _: &Self::Socket,
        _: &[IoSlice<'_>],
        _: Option<SocketAddress>,
        _: SocketSendFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        cancellation.check()?;
        Ok(0)
    }
}
impl SocketWait for Sockets {
    async fn socket_wait(
        &self,
        socket: &Self::Socket,
        _: Interest,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        cancellation.check()?;
        match socket.load(Ordering::Relaxed) {
            6 => {
                cancellation.cancelled().await;
                Err(Cancelled.into())
            }
            7 => {
                socket.store(1, Ordering::Relaxed);
                Ok(())
            }
            8 => Err(io::Error::from(io::ErrorKind::BrokenPipe).into()),
            9 => {
                cancellation.cancel();
                Err(Cancelled.into())
            }
            10 => Err(io::Error::from(io::ErrorKind::InvalidInput).into()),
            12 => {
                socket.store(6, Ordering::Relaxed);
                Ok(())
            }
            _ => Err(io::Error::from(io::ErrorKind::WouldBlock).into()),
        }
    }
}
impl SocketMonitoring for Sockets {
    type MonitorRegistration = ();
    async fn socket_monitor_close<M: SocketMonitorState<Self::Socket, ()>>(
        &self,
        monitor: &mut MonitorStorage<M>,
    ) -> io::Result<()> {
        match monitor.as_ref().socket().load(Ordering::Relaxed) {
            100 => pending().await,
            101 => Err(io::ErrorKind::InvalidInput.into()),
            102 => Err(io::ErrorKind::BrokenPipe.into()),
            _ => Ok(()),
        }
    }
    fn socket_monitor_state(
        &self,
        socket: &Self::Socket,
        interest: Interest,
    ) -> io::Result<SocketMonitor<Self::Socket, ()>> {
        Ok(SocketMonitor::new(socket.clone(), interest, ()))
    }
}

#[test]
fn close_abandonment_and_errors_never_restart_reception() {
    block_on(async {
        let never = Cancellation::never();
        let socket = Arc::new(AtomicUsize::new(1));
        let pool = TestPool(HeapBufferPool::new(1, 8).unwrap());
        let mut receiver = SocketReceiver::new(
            Sockets.socket_monitor(&socket, Interest::Readable).unwrap(),
            pool.clone(),
            SocketType::Stream,
            SocketRecvFlags::default(),
        )
        .unwrap();
        socket.store(1, Ordering::Relaxed);
        let held = receiver.recv(&Sockets, &never).await.unwrap().unwrap();
        assert_eq!(held.payload(), &[42; 8]);
        assert!(pool.try_acquire().is_none());
        socket.store(100, Ordering::Relaxed);
        {
            let mut close = pin!(receiver.close(&Sockets));
            assert!(poll_once(close.as_mut()).await.is_none());
        }
        // It must not wait for the held slot or touch a closing monitor.
        let result = poll_once(receiver.recv(&Sockets, &never))
            .await
            .unwrap()
            .unwrap();
        assert!(result.is_none());
        socket.store(102, Ordering::Relaxed);
        assert_eq!(
            receiver.close(&Sockets).await.unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        socket.store(101, Ordering::Relaxed);
        assert!(receiver.close(&Sockets).await.is_err());
        assert!(receiver.recv(&Sockets, &never).await.unwrap().is_none());
        socket.store(1, Ordering::Relaxed);
        receiver.close(&Sockets).await.unwrap();
        receiver.close(&Sockets).await.unwrap();
        assert_eq!(held.payload(), &[42; 8]);
    });
}
fn receiver(
    socket: Arc<AtomicUsize>,
    kind: SocketType,
) -> SocketReceiver<SocketMonitor<Arc<AtomicUsize>, ()>, TestPool> {
    SocketReceiver::new(
        Sockets.socket_monitor(&socket, Interest::Readable).unwrap(),
        TestPool(HeapBufferPool::new(2, 8).unwrap()),
        kind,
        SocketRecvFlags::default(),
    )
    .unwrap()
}

#[test]
fn receiver_preserves_retryable_rejections_and_distinguishes_cancellation_phases() {
    block_on(async {
        let socket = Arc::new(AtomicUsize::new(2));
        let never = Cancellation::never();
        let mut receiver = receiver(socket.clone(), SocketType::Stream);
        for state in [2, 3] {
            socket.store(state, Ordering::Relaxed);
            assert!(receiver.recv(&Sockets, &never).await.is_err());
        }
        socket.store(1, Ordering::Relaxed);
        let cancelled = Cancellation::new();
        cancelled.cancel();
        assert!(receiver.recv(&Sockets, &cancelled).await.is_err());
        drop(receiver.close(&Sockets));
        let message = receiver.recv(&Sockets, &never).await.unwrap().unwrap();
        let frozen = ReceiveBuffer::freeze(message.into_payload());
        assert_eq!(frozen.as_ref(), [42; 8]);
        socket.store(5, Ordering::Relaxed);
        assert!(receiver.recv(&Sockets, &Cancellation::new()).await.is_err());
        assert!(receiver.recv(&Sockets, &never).await.unwrap().is_none());
        receiver.close(&Sockets).await.unwrap();
    });
}

#[test]
fn receiver_reports_terminal_errors_once_and_close_keeps_completed_chunks_alive() {
    block_on(async {
        let socket = Arc::new(AtomicUsize::new(4));
        let never = Cancellation::never();
        let mut receiver = receiver(socket.clone(), SocketType::Stream);
        assert!(receiver.recv(&Sockets, &never).await.is_err());
        assert!(receiver.recv(&Sockets, &never).await.unwrap().is_none());
        SocketRecvMsgReceiver::close(&mut receiver, &Sockets)
            .await
            .unwrap();
        let mut packets = super_receiver(socket.clone());
        socket.store(0, Ordering::Relaxed);
        let message = packets.recv(&Sockets, &never).await.unwrap().unwrap();
        assert_eq!(message.payload(), b"");
        packets.close(&Sockets).await.unwrap();
        drop(packets);
        assert_eq!(message.payload(), b"");
    });
}
fn super_receiver(
    socket: Arc<AtomicUsize>,
) -> SocketReceiver<SocketMonitor<Arc<AtomicUsize>, ()>, TestPool> {
    receiver(socket, SocketType::Datagram)
}

#[test]
fn receiver_rejects_peeking_without_consuming_a_message() {
    let socket = Arc::new(AtomicUsize::new(1));
    assert!(
        SocketReceiver::new(
            Sockets.socket_monitor(&socket, Interest::Readable).unwrap(),
            TestPool(HeapBufferPool::new(1, 8).unwrap()),
            SocketType::Stream,
            SocketRecvFlags { peek: true }
        )
        .is_err()
    );
}

#[test]
fn idle_receiver_returns_capacity_for_another_connection() {
    block_on(async {
        let pool = TestPool(HeapBufferPool::new(1, 8).unwrap());
        let never = Cancellation::never();
        let idle_socket = Arc::new(AtomicUsize::new(6));
        let active_socket = Arc::new(AtomicUsize::new(1));
        let mut idle = SocketReceiver::new(
            Sockets
                .socket_monitor(&idle_socket, Interest::Readable)
                .unwrap(),
            pool.clone(),
            SocketType::Stream,
            SocketRecvFlags::default(),
        )
        .unwrap();
        let mut active = SocketReceiver::new(
            Sockets
                .socket_monitor(&active_socket, Interest::Readable)
                .unwrap(),
            pool.clone(),
            SocketType::Stream,
            SocketRecvFlags::default(),
        )
        .unwrap();
        {
            let mut waiting = pin!(idle.recv(&Sockets, &never));
            assert!(poll_once(waiting.as_mut()).await.is_none());
            // The idle receive is still alive and pending. Its slot can serve an active connection.
            let message = active.recv(&Sockets, &never).await.unwrap().unwrap();
            assert_eq!(message.payload(), [42; 8]);
            assert!(pool.try_acquire().is_none());
            drop(message);
            assert!(pool.try_acquire().is_some());
        }
        idle.close(&Sockets).await.unwrap();
        active.close(&Sockets).await.unwrap();
        assert!(pool.try_acquire().is_some());
    });
}

#[test]
fn receiver_retries_after_readiness_without_retaining_an_old_buffer() {
    block_on(async {
        let socket = Arc::new(AtomicUsize::new(7));
        let mut receiver = receiver(socket, SocketType::Stream);
        let never = Cancellation::never();
        let message = receiver.recv(&Sockets, &never).await.unwrap().unwrap();
        assert_eq!(message.payload(), [42; 8]);
        receiver.close(&Sockets).await.unwrap();
    });
}

#[test]
fn advisory_readiness_releases_capacity_again_and_cancellation_retires_the_wait() {
    block_on(async {
        let pool = TestPool(HeapBufferPool::new(1, 8).unwrap());
        // The first readiness wake is advisory: no message is available on the retry.
        let socket = Arc::new(AtomicUsize::new(12));
        let mut receiver = SocketReceiver::new(
            Sockets.socket_monitor(&socket, Interest::Readable).unwrap(),
            pool.clone(),
            SocketType::Datagram,
            SocketRecvFlags::default(),
        )
        .unwrap();
        let cancellation = Cancellation::new();
        {
            let mut waiting = pin!(receiver.recv(&Sockets, &cancellation));
            assert!(poll_once(waiting.as_mut()).await.is_none());
            assert!(pool.try_acquire().is_some());
            cancellation.cancel();
            assert!(matches!(waiting.await, Err(OperationError::Cancelled(_))));
        }
        let never = Cancellation::never();
        assert!(receiver.recv(&Sockets, &never).await.unwrap().is_none());
        SocketRecvMsgReceiver::close(&mut receiver, &Sockets)
            .await
            .unwrap();
        assert!(pool.try_acquire().is_some());
    });
}

#[test]
fn readiness_failures_follow_receive_error_policy() {
    block_on(async {
        let never = Cancellation::never();
        for state in [8, 9, 10, 11] {
            let socket = Arc::new(AtomicUsize::new(state));
            let mut receiver = receiver(socket.clone(), SocketType::Stream);
            let cancellation = Cancellation::new();
            let error = receiver.recv(&Sockets, &cancellation).await.err().unwrap();
            assert_eq!(
                error.kind(),
                match state {
                    8 => io::ErrorKind::BrokenPipe,
                    9 => io::ErrorKind::Interrupted,
                    10 => io::ErrorKind::InvalidInput,
                    _ => io::ErrorKind::WouldBlock,
                }
            );
            socket.store(1, Ordering::Relaxed);
            let next = receiver.recv(&Sockets, &never).await.unwrap();
            assert_eq!(next.is_some(), state >= 10);
            SocketRecvMsgReceiver::close(&mut receiver, &Sockets)
                .await
                .unwrap();
        }
    });
}
