// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::*;
use core_scheduler_direct::DirectScheduler;
use core_scheduler_examples::{datagrams, frames};
use futures_lite::future::block_on;
use std::{
    collections::VecDeque,
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

enum Packet {
    Data(&'static [u8]),
    Truncated,
    Eof,
    Failed,
    WaitForCancellation,
}

struct Receives {
    executor: DirectScheduler,
    packets: Arc<Mutex<VecDeque<Packet>>>,
    closed: Arc<AtomicUsize>,
    reject_spawn: bool,
    fail_close: bool,
}

impl Receives {
    fn new(packets: impl IntoIterator<Item = Packet>) -> Self {
        Self {
            executor: DirectScheduler::new(Default::default()).unwrap(),
            packets: Arc::new(Mutex::new(packets.into_iter().collect())),
            closed: Arc::new(AtomicUsize::new(0)),
            reject_spawn: false,
            fail_close: false,
        }
    }
}
impl SocketOperation for Receives {
    type Socket = SocketType;
}
impl BufferPooling for Receives {
    type Pool = HeapBufferPool;
    fn buffer_pool(&self, options: BufferPoolOptions) -> io::Result<Self::Pool> {
        HeapBufferPool::with_options(options)
    }
}
impl Spawn for Receives {
    type Handle<T: Send + 'static> = <DirectScheduler as Spawn>::Handle<T>;
    fn spawn<F: std::future::Future + Send + 'static>(
        &self,
        future: F,
    ) -> Result<Self::Handle<F::Output>, SpawnError>
    where
        F::Output: Send + 'static,
    {
        if self.reject_spawn {
            Err(SpawnError::SchedulerClosed)
        } else {
            self.executor.spawn(future)
        }
    }
}
impl SocketRecvPoll for Receives {
    async fn socket_recv_poll(
        &self,
        socket: &SocketType,
        buffer: &mut [u8],
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.socket_recvmsg_poll(
            socket,
            &mut [io::IoSliceMut::new(buffer)],
            flags,
            cancellation,
        )
        .await
        .map(|result| result.bytes)
    }
}
impl SocketSendPoll for Receives {
    async fn socket_send_poll(
        &self,
        _: &SocketType,
        _: &[u8],
        _: SocketSendFlags,
        _: &Cancellation,
    ) -> OperationResult<usize> {
        unreachable!("receive examples do not send")
    }
}
impl SocketSendMsgPoll for Receives {
    async fn socket_sendmsg_poll(
        &self,
        _: &SocketType,
        _: &[io::IoSlice<'_>],
        _: Option<SocketAddress>,
        _: SocketSendFlags,
        _: &Cancellation,
    ) -> OperationResult<usize> {
        unreachable!("receive examples do not send")
    }
}
impl SocketRecvMsgPoll for Receives {
    async fn socket_recvmsg_poll(
        &self,
        _: &SocketType,
        buffers: &mut [io::IoSliceMut<'_>],
        _: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<SocketRecvResult> {
        cancellation.check()?;
        let packet = self
            .packets
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Packet::Eof);
        let (bytes, truncated) = match packet {
            Packet::Eof => (&b""[..], false),
            Packet::Failed => return Err(io::Error::from(io::ErrorKind::ConnectionReset).into()),
            Packet::WaitForCancellation => {
                cancellation.cancelled().await;
                return Err(Cancelled.into());
            }
            Packet::Data(bytes) => (bytes, false),
            Packet::Truncated => (&b""[..], true),
        };
        let mut copied = 0;
        for buffer in buffers {
            let count = buffer.len().min(bytes.len() - copied);
            buffer[..count].copy_from_slice(&bytes[copied..copied + count]);
            copied += count;
        }
        Ok(SocketRecvResult {
            bytes: copied,
            address: Some("127.0.0.1:1234".parse().unwrap()),
            truncated: truncated || copied < bytes.len(),
            control_truncated: false,
        })
    }
}
impl SocketWait for Receives {
    async fn socket_wait(
        &self,
        _: &SocketType,
        _: Interest,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        cancellation.check()?;
        if !self.packets.lock().unwrap().is_empty() {
            return Ok(());
        }
        cancellation.cancelled().await;
        Err(Cancelled.into())
    }
}
impl SocketMonitoring for Receives {
    type MonitorRegistration = ();
    fn socket_type(&self, socket: &SocketType) -> io::Result<SocketType> {
        Ok(*socket)
    }
    fn socket_monitor_state(
        &self,
        socket: &SocketType,
        interest: Interest,
    ) -> io::Result<SocketMonitor<SocketType, ()>> {
        Ok(SocketMonitor::new(*socket, interest, ()))
    }
    async fn socket_monitor_close<M: SocketMonitorState<SocketType, ()>>(
        &self,
        _: &mut MonitorStorage<M>,
    ) -> io::Result<()> {
        self.closed.fetch_add(1, Ordering::Relaxed);
        if self.fail_close {
            Err(io::ErrorKind::BrokenPipe.into())
        } else {
            Ok(())
        }
    }
}

#[test]
fn packet_handoff_preserves_empty_messages_addresses_and_bounded_capacity() {
    block_on(async {
        let scheduler = Receives::new([
            Packet::Data(b""),
            Packet::Data(b"payload"),
            Packet::Data(b"last"),
        ]);
        let pool = HeapBufferPool::new(1, 16).unwrap();
        let processed = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&processed);
        datagrams::receive(
            &scheduler,
            scheduler
                .socket_packet_reader_with_pool(&SocketType::Datagram, pool.clone())
                .unwrap(),
            3,
            2,
            &Cancellation::never(),
            move |message| {
                assert_eq!(message.address().unwrap().to_socket_address().port(), 1234);
                assert!([&b""[..], &b"payload"[..], &b"last"[..]].contains(&message.payload()));
                counter.fetch_add(1, Ordering::Relaxed);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(processed.load(Ordering::Relaxed), 3);
        assert_eq!(scheduler.closed.load(Ordering::Relaxed), 1);
        assert!(pool.try_acquire().is_some());
        scheduler.executor.shutdown().await;
    });
}

#[test]
fn packet_errors_and_task_panics_still_close_receivers_and_drain_accepted_work() {
    block_on(async {
        for mode in 0..8 {
            let packet = match mode {
                0 => Packet::Failed,
                1 => Packet::Truncated,
                _ => Packet::Data(b"a"),
            };
            let mut scheduler = Receives::new([packet, Packet::Data(b"b")]);
            scheduler.reject_spawn = mode == 2;
            scheduler.fail_close = mode == 3;
            let pool = HeapBufferPool::new(2, 16).unwrap();
            let error = datagrams::receive(
                &scheduler,
                scheduler
                    .socket_packet_reader_with_pool(&SocketType::Datagram, pool.clone())
                    .unwrap(),
                if mode == 5 || mode == 7 { 2 } else { 1 },
                1,
                &Cancellation::never(),
                move |_| {
                    if mode >= 6 {
                        panic!("handler failed");
                    }
                    if mode >= 4 {
                        return Err(io::Error::from(io::ErrorKind::InvalidData).into());
                    }
                    Ok(())
                },
            )
            .await
            .unwrap_err();
            assert_eq!(
                error.kind(),
                match mode {
                    0 => io::ErrorKind::ConnectionReset,
                    1 | 4 | 5 => io::ErrorKind::InvalidData,
                    3 => io::ErrorKind::BrokenPipe,
                    _ => io::ErrorKind::Other,
                }
            );
            assert_eq!(scheduler.closed.load(Ordering::Relaxed), 1);
            let a = pool.try_acquire().unwrap();
            let b = pool.try_acquire().unwrap();
            drop((a, b));
            scheduler.executor.shutdown().await;
        }
    });
}

#[test]
fn cancellation_after_handoff_waits_for_processing_and_returns_storage() {
    block_on(async {
        let scheduler = Receives::new([Packet::Data(b"a"), Packet::WaitForCancellation]);
        let token = Cancellation::new();
        let notify = token.clone();
        let pool = HeapBufferPool::new(2, 16).unwrap();
        assert!(matches!(
            datagrams::receive(
                &scheduler,
                scheduler
                    .socket_packet_reader_with_pool(&SocketType::Datagram, pool.clone())
                    .unwrap(),
                2,
                2,
                &token,
                move |_| {
                    notify.cancel();
                    Ok(())
                }
            )
            .await,
            Err(OperationError::Cancelled(_))
        ));
        assert_eq!(scheduler.closed.load(Ordering::Relaxed), 1);
        assert!(pool.try_acquire().is_some());
        assert!(
            datagrams::receive(
                &scheduler,
                scheduler
                    .socket_packet_reader_with_pool(&SocketType::Datagram, pool.clone())
                    .unwrap(),
                1,
                0,
                &token,
                |_| Ok(())
            )
            .await
            .is_err()
        );
        assert!(
            datagrams::receive(
                &scheduler,
                scheduler
                    .socket_packet_reader_with_pool(&SocketType::Datagram, pool)
                    .unwrap(),
                1,
                1,
                &token,
                |_| Ok(())
            )
            .await
            .is_err()
        );
        scheduler.executor.shutdown().await;
    });
}

#[test]
fn frame_receiver_closes_after_incomplete_eof_consumer_failure_and_cancellation() {
    block_on(async {
        for mode in 0..3 {
            let scheduler = Receives::new([
                Packet::Data(b"a"),
                if mode == 2 {
                    Packet::WaitForCancellation
                } else {
                    Packet::Eof
                },
            ]);
            let cancellation = Cancellation::new();
            let result = frames::receive(
                &scheduler,
                &SocketType::Stream,
                HeapBufferPool::new(2, 16).unwrap(),
                if mode == 0 { 2 } else { 1 },
                &cancellation,
                |_| {
                    if mode == 2 {
                        cancellation.cancel();
                        Ok(())
                    } else {
                        Err(io::Error::from(io::ErrorKind::InvalidData).into())
                    }
                },
            )
            .await;
            assert_eq!(
                result.unwrap_err().kind(),
                match mode {
                    0 => io::ErrorKind::UnexpectedEof,
                    1 => io::ErrorKind::InvalidData,
                    _ => io::ErrorKind::Interrupted,
                }
            );
            assert_eq!(scheduler.closed.load(Ordering::Relaxed), 1);
            scheduler.executor.shutdown().await;
        }
    });
}

#[test]
fn frame_receiver_validates_setup_and_reports_close_failure() {
    block_on(async {
        for mode in 0..3 {
            let mut scheduler = Receives::new([]);
            scheduler.fail_close = mode == 2;
            let token = Cancellation::new();
            if mode == 1 {
                token.cancel();
            }
            let result = frames::receive(
                &scheduler,
                &SocketType::Stream,
                HeapBufferPool::new(1, 16).unwrap(),
                if mode == 0 { 0 } else { 1 },
                &token,
                |_| Ok(()),
            )
            .await;
            assert_eq!(
                result.unwrap_err().kind(),
                match mode {
                    0 => io::ErrorKind::InvalidInput,
                    1 => io::ErrorKind::Interrupted,
                    _ => io::ErrorKind::BrokenPipe,
                }
            );
            assert_eq!(
                scheduler.closed.load(Ordering::Relaxed),
                usize::from(mode == 2)
            );
            scheduler.executor.shutdown().await;
        }
    });
}

impl SocketStreamReading for Receives {}
impl SocketPacketReading for Receives {}
