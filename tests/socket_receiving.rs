// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::backend::*;
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
    fn socket_type(&self, socket: &Self::Socket) -> io::Result<SocketType> {
        match socket.load(Ordering::Relaxed) {
            200 => Ok(SocketType::Datagram),
            201 => Err(io::ErrorKind::PermissionDenied.into()),
            _ => Ok(SocketType::Stream),
        }
    }
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
        if socket.load(Ordering::Relaxed) == 202 {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
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

thread_local! { static FAIL_POOL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

impl BufferPooling for Sockets {
    type Pool = HeapBufferPool;
    fn buffer_pool(&self, options: BufferPoolOptions) -> io::Result<Self::Pool> {
        if FAIL_POOL.with(|flag| flag.replace(false)) {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        HeapBufferPool::with_options(options)
    }
}

#[test]
fn bound_stream_reader_preserves_borrowing_eof_and_owned_chunks() {
    block_on(async {
        let socket = Arc::new(AtomicUsize::new(1));
        let never = Cancellation::never();
        let mut reader = BoundSocketStreamReader::new(&Sockets, &socket).unwrap();
        assert_eq!(reader.read_into(&mut [], &never).await.unwrap(), 0);
        let mut bytes = [0; 4];
        assert_eq!(reader.read_into(&mut bytes, &never).await.unwrap(), 4);
        assert_eq!(bytes, [42; 4]);
        let mut vectors = [IoSliceMut::new(&mut bytes)];
        assert_eq!(reader.read_vectored(&mut vectors, &never).await.unwrap(), 4);
        let owned = reader.read(&never).await.unwrap().unwrap();
        assert_eq!(owned.as_ref().len(), 64 * 1024);
        let pool = HeapBufferPool::new(1, 8).unwrap();
        reader = reader.with_pool(pool.clone());
        let owned = reader.read(&never).await.unwrap().unwrap();
        assert_eq!(owned.as_ref(), [42; 8]);
        socket.store(0, Ordering::Relaxed);
        drop(owned);
        assert_eq!(reader.read_into(&mut bytes, &never).await.unwrap(), 0);
        assert!(reader.read(&never).await.unwrap().is_none());
        assert_eq!(
            reader
                .read_vectored(&mut [IoSliceMut::new(&mut bytes)], &never)
                .await
                .unwrap(),
            0
        );
        reader.close().await.unwrap();
        assert_eq!(
            reader
                .read_into(&mut bytes, &never)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
        reader.close().await.unwrap();
    });
}

#[test]
fn bound_stream_reader_empty_vectors_do_not_establish_eof() {
    block_on(async {
        let socket = Arc::new(AtomicUsize::new(0));
        let never = Cancellation::never();
        let mut reader = Sockets.socket_stream_reader(&socket).unwrap();
        let mut empty = [];
        assert_eq!(
            reader
                .read_vectored(&mut [IoSliceMut::new(&mut empty)], &never)
                .await
                .unwrap(),
            0
        );
        socket.store(1, Ordering::Relaxed);
        assert!(reader.read(&never).await.unwrap().is_some());
        socket.store(0, Ordering::Relaxed);
        assert!(reader.read(&never).await.unwrap().is_none());
        assert_eq!(reader.read_into(&mut [0; 1], &never).await.unwrap(), 0);
        reader.close().await.unwrap();
    });
}

#[test]
fn bound_stream_reader_returns_idle_capacity_and_retries_cancelled_waits() {
    block_on(async {
        let socket = Arc::new(AtomicUsize::new(6));
        let never = Cancellation::never();
        let pool = HeapBufferPool::new(1, 8).unwrap();
        let mut reader = Sockets
            .socket_stream_reader_with_pool(&socket, pool.clone())
            .unwrap();
        let cancellation = Cancellation::new();
        {
            let mut reading = pin!(reader.read(&cancellation));
            assert!(poll_once(reading.as_mut()).await.is_none());
            assert!(pool.try_acquire().is_some());
            cancellation.cancel();
            assert!(reading.await.is_err());
        }
        socket.store(7, Ordering::Relaxed);
        let chunk = reader.read(&never).await.unwrap().unwrap();
        assert_eq!(chunk.as_ref(), [42; 8]);
        {
            let mut reading = pin!(reader.read(&cancellation));
            assert!(poll_once(reading.as_mut()).await.unwrap().is_err());
        }
        reader.close().await.unwrap();
        assert_eq!(chunk.as_ref(), [42; 8]);
        assert!(reader.read(&never).await.unwrap().is_none());
    });
}

#[test]
fn bound_stream_reader_errors_and_abandoned_close_do_not_reopen_input() {
    block_on(async {
        let socket = Arc::new(AtomicUsize::new(4));
        let never = Cancellation::never();
        let mut reader = Sockets.socket_stream_reader(&socket).unwrap();
        assert!(reader.read(&never).await.is_err());
        assert!(reader.read_into(&mut [0; 1], &never).await.is_err());
        assert!(
            reader
                .read_vectored(&mut [IoSliceMut::new(&mut [0; 1])], &never)
                .await
                .is_err()
        );
        socket.store(8, Ordering::Relaxed);
        assert!(reader.read(&never).await.is_err());
        socket.store(100, Ordering::Relaxed);
        {
            let mut close = pin!(reader.close());
            assert!(poll_once(close.as_mut()).await.is_none());
        }
        assert!(reader.read(&never).await.unwrap().is_none());
        assert!(reader.read_vectored(&mut [], &never).await.is_err());
        socket.store(102, Ordering::Relaxed);
        assert!(reader.close().await.is_err());
        socket.store(1, Ordering::Relaxed);
        reader.close().await.unwrap();
    });
}

#[test]
fn bound_stream_reader_rejects_invalid_socket_kinds_and_setup_failures() {
    for state in [200, 201, 202] {
        let socket = Arc::new(AtomicUsize::new(state));
        assert!(Sockets.socket_stream_reader(&socket).is_err());
        assert!(
            Sockets
                .socket_stream_reader_with_pool(&socket, HeapBufferPool::new(1, 8).unwrap())
                .is_err()
        );
    }
}

struct Output<B> {
    socket: Arc<AtomicUsize>,
    marker: std::marker::PhantomData<B>,
}
impl SocketStreamOutputting for Sockets {
    type Writer<B: IoBuf> = Output<B>;
    fn socket_stream_output<B: IoBuf>(&self, socket: &Self::Socket) -> io::Result<Self::Writer<B>> {
        if socket.load(Ordering::Relaxed) == 200 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(Output {
            socket: socket.clone(),
            marker: std::marker::PhantomData,
        })
    }
}
impl<B: IoBuf> SocketStreamOutput<Sockets> for Output<B> {
    type Buffer = B;
    async fn submit(
        &mut self,
        _: &Sockets,
        buffer: B,
        cancellation: &Cancellation,
    ) -> Result<(), (OperationError, B)> {
        if let Err(error) = cancellation.check() {
            return Err((error.into(), buffer));
        }
        if self.socket.load(Ordering::Relaxed) == 105 {
            std::future::pending::<()>().await;
        }
        Ok(())
    }
    async fn flush(&mut self, _: &Sockets, cancellation: &Cancellation) -> OperationResult<()> {
        cancellation.check()?;
        if self.socket.load(Ordering::Relaxed) == 104 {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe).into());
        }
        Ok(())
    }
    async fn close(&mut self, _: &Sockets) -> io::Result<()> {
        if self.socket.load(Ordering::Relaxed) == 103 {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        Ok(())
    }
}

#[test]
fn bound_stream_writer_combines_borrowed_and_owned_output() {
    block_on(async {
        let never = Cancellation::never();
        let socket = Arc::new(AtomicUsize::new(1));
        let mut writer = Sockets.socket_stream_writer(&socket).unwrap();
        assert_eq!(writer.write(b"borrowed", &never).await.unwrap(), 8);
        assert_eq!(
            writer
                .write_vectored(&[IoSlice::new(b"vector")], &never)
                .await
                .unwrap(),
            0
        );
        let pool = HeapBufferPool::new(1, 8).unwrap();
        writer
            .submit(pool.acquire(&never).await.unwrap(), &never)
            .await
            .map_err(|(e, _)| e)
            .unwrap();
        assert_eq!(writer.write(b"next", &never).await.unwrap(), 4);
        writer
            .submit(pool.acquire(&never).await.unwrap(), &never)
            .await
            .map_err(|(e, _)| e)
            .unwrap();
        assert_eq!(writer.write_vectored(&[], &never).await.unwrap(), 0);
        writer.flush(&never).await.unwrap();
        drop(writer.close());
        writer.close().await.unwrap();
        writer.close().await.unwrap();
        assert!(writer.write(b"closed", &never).await.is_err());
        assert!(writer.write_vectored(&[], &never).await.is_err());
        assert!(writer.flush(&never).await.is_err());
        let returned = pool.acquire(&never).await.unwrap();
        let address = returned.as_ref().as_ptr();
        let (error, returned) = writer.submit(returned, &never).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(returned.as_ref().as_ptr(), address);
        drop(returned);
    });
}

#[test]
fn bound_stream_writer_preserves_ordering_after_abandoned_admission_and_errors() {
    block_on(async {
        let never = Cancellation::never();
        let socket = Arc::new(AtomicUsize::new(105));
        let mut writer = Sockets
            .socket_stream_writer_with_buffer::<Vec<u8>>(&socket)
            .unwrap();
        {
            let mut submitting = pin!(writer.submit(b"pending".to_vec(), &never));
            assert!(poll_once(submitting.as_mut()).await.is_none());
        }
        socket.store(104, Ordering::Relaxed);
        assert!(writer.write(b"next", &never).await.is_err());
        assert!(writer.write_vectored(&[], &never).await.is_err());
        assert!(writer.flush(&never).await.is_err());
        socket.store(103, Ordering::Relaxed);
        assert!(writer.close().await.is_err());
        socket.store(102, Ordering::Relaxed);
        assert!(writer.close().await.is_err());
        socket.store(1, Ordering::Relaxed);
        writer.close().await.unwrap();
        socket.store(200, Ordering::Relaxed);
        assert!(Sockets.socket_stream_writer(&socket).is_err());
        socket.store(202, Ordering::Relaxed);
        assert!(Sockets.socket_stream_writer(&socket).is_err());
    });
}

#[test]
fn bound_stream_reader_preserves_pool_failures_exhaustion_and_cancellation_at_eof() {
    block_on(async {
        let socket = Arc::new(AtomicUsize::new(1));
        let never = Cancellation::never();
        let mut reader = BoundSocketStreamReader::new(&Sockets, &socket).unwrap();
        FAIL_POOL.with(|flag| flag.set(true));
        assert_eq!(
            reader.read(&never).await.unwrap_err().kind(),
            io::ErrorKind::OutOfMemory
        );
        let pool = HeapBufferPool::new(1, 1).unwrap();
        let held = pool.try_acquire().unwrap();
        reader = reader.with_pool(pool);
        let cancellation = Cancellation::new();
        {
            let mut reading = pin!(reader.read(&cancellation));
            assert!(poll_once(reading.as_mut()).await.is_none());
            cancellation.cancel();
            assert!(reading.await.is_err());
        }
        drop(held);
        socket.store(0, Ordering::Relaxed);
        assert_eq!(
            reader
                .read_vectored(&mut [IoSliceMut::new(&mut [0; 1])], &never)
                .await
                .unwrap(),
            0
        );
        assert!(reader.read_into(&mut [0; 1], &cancellation).await.is_err());
        assert!(reader.read_vectored(&mut [], &cancellation).await.is_err());
        reader.close().await.unwrap();
    });
}

#[test]
fn opaque_stream_factories_bind_the_scheduler_without_borrowing_the_input_reference() {
    block_on(async {
        let never = Cancellation::never();
        let mut reader = {
            let socket = Arc::new(AtomicUsize::new(1));
            Sockets.socket_stream_reader(&socket).unwrap()
        };
        let chunk = reader.read(&never).await.unwrap().unwrap();
        let mut writer = {
            let socket = Arc::new(AtomicUsize::new(1));
            Sockets.socket_stream_writer(&socket).unwrap()
        };
        writer.submit(chunk, &never).await.unwrap();
        writer.flush(&never).await.unwrap();
        writer.close().await.unwrap();
        reader.close().await.unwrap();
        let mut writer = {
            let socket = Arc::new(AtomicUsize::new(1));
            Sockets
                .socket_stream_writer_with_buffer::<Vec<u8>>(&socket)
                .unwrap()
        };
        writer.submit(b"typed".to_vec(), &never).await.unwrap();
        writer.close().await.unwrap();
    });
}

impl SocketStreamWriting for Sockets {
    fn socket_stream_writer<'scheduler>(
        &'scheduler self,
        socket: &Self::Socket,
    ) -> io::Result<impl SocketStreamWriter<Buffer = <Self::Pool as BufferPool>::Buffer> + 'scheduler>
    where
        Self::Socket: 'static,
    {
        BoundSocketStreamWriter::new(self, socket)
    }

    fn socket_stream_writer_with_buffer<'scheduler, B: IoBuf>(
        &'scheduler self,
        socket: &Self::Socket,
    ) -> io::Result<impl SocketStreamWriter<Buffer = B> + 'scheduler>
    where
        Self::Socket: 'static,
    {
        BoundSocketStreamWriter::new(self, socket)
    }
}

impl SocketStreamReading for Sockets {
    fn socket_stream_reader<'scheduler>(
        &'scheduler self,
        socket: &Self::Socket,
    ) -> io::Result<impl SocketStreamReader<Buffer = <Self::Pool as BufferPool>::Buffer> + 'scheduler>
    {
        BoundSocketStreamReader::new(self, socket)
    }
    fn socket_stream_reader_with_pool<'scheduler>(
        &'scheduler self,
        socket: &Self::Socket,
        pool: Self::Pool,
    ) -> io::Result<impl SocketStreamReader<Buffer = <Self::Pool as BufferPool>::Buffer> + 'scheduler>
    {
        Ok(BoundSocketStreamReader::new(self, socket)?.with_pool(pool))
    }
}
