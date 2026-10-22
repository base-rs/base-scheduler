// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::*;
use futures_lite::future::block_on;
use std::{
    io::{self, IoSlice, IoSliceMut},
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
            _ => {
                cancellation.cancel();
                Err(Cancelled.into())
            }
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
        _: &Self::Socket,
        _: Interest,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        cancellation.check()?;
        Ok(())
    }
}
impl SocketMonitoring for Sockets {
    type MonitorRegistration = ();
    fn socket_monitor_state(
        &self,
        socket: &Self::Socket,
        interest: Interest,
    ) -> io::Result<SocketMonitor<Self::Socket, ()>> {
        Ok(SocketMonitor::new(socket.clone(), interest, ()))
    }
}
fn receiver(
    socket: Arc<AtomicUsize>,
    kind: SocketType,
) -> SocketReceiver<SocketMonitor<Arc<AtomicUsize>, ()>> {
    SocketReceiver::new(
        Sockets.socket_monitor(&socket, Interest::Readable).unwrap(),
        BufferPool::new(2, 8).unwrap(),
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
        assert!(receiver.close(&Sockets, &cancelled).await.is_err());
        let message = receiver.recv(&Sockets, &never).await.unwrap().unwrap();
        let frozen = ReceiveBuffer::freeze(message.buffer);
        assert_eq!(frozen.as_ref(), [42; 8]);
        socket.store(5, Ordering::Relaxed);
        assert!(receiver.recv(&Sockets, &Cancellation::new()).await.is_err());
        assert!(receiver.recv(&Sockets, &never).await.unwrap().is_none());
        receiver.close(&Sockets, &never).await.unwrap();
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
        assert_eq!(message.buffer.as_ref(), b"");
        packets.close(&Sockets, &never).await.unwrap();
        drop(packets);
        assert_eq!(message.buffer.as_ref(), b"");
    });
}
fn super_receiver(socket: Arc<AtomicUsize>) -> SocketReceiver<SocketMonitor<Arc<AtomicUsize>, ()>> {
    receiver(socket, SocketType::Datagram)
}

#[test]
fn receiver_rejects_peeking_without_consuming_a_message() {
    let socket = Arc::new(AtomicUsize::new(1));
    assert!(
        SocketReceiver::new(
            Sockets.socket_monitor(&socket, Interest::Readable).unwrap(),
            BufferPool::new(1, 8).unwrap(),
            SocketType::Stream,
            SocketRecvFlags { peek: true }
        )
        .is_err()
    );
}
