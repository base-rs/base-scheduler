// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::*;
use futures_lite::future::{block_on, poll_once};
use std::{
    collections::VecDeque,
    future::{Future, pending},
    io::{self, IoSlice, IoSliceMut},
    pin::pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};

#[path = "support/buffer_pool.rs"]
mod pool_support;
use pool_support::TestPool;

struct Packet {
    bytes: Vec<u8>,
    address: Option<SocketAddress>,
    control_truncated: bool,
}

struct Socket {
    kind: SocketType,
    incoming: Mutex<VecDeque<Packet>>,
    send: Mutex<([u8; 32], usize, Option<SocketAddress>)>,
    error: Mutex<Option<io::ErrorKind>>,
    waits: AtomicUsize,
    sends: AtomicUsize,
    pending_close: AtomicBool,
    fail_close: AtomicBool,
    pending_send: AtomicBool,
    partial_send: AtomicBool,
    receive_on_wait: AtomicBool,
}

impl Socket {
    fn new(kind: SocketType) -> Arc<Self> {
        Arc::new(Self {
            kind,
            incoming: Mutex::default(),
            send: Mutex::new(([0; 32], 0, None)),
            error: Mutex::default(),
            waits: AtomicUsize::new(0),
            sends: AtomicUsize::new(0),
            pending_close: AtomicBool::new(false),
            fail_close: AtomicBool::new(false),
            pending_send: AtomicBool::new(false),
            partial_send: AtomicBool::new(false),
            receive_on_wait: AtomicBool::new(false),
        })
    }

    fn packet(&self, bytes: &[u8]) {
        self.incoming.lock().unwrap().push_back(Packet {
            bytes: bytes.to_vec(),
            address: Some("127.0.0.1:4567".parse().unwrap()),
            control_truncated: true,
        });
    }
}

#[derive(Default)]
struct Scheduler {
    pools: AtomicUsize,
    options: Mutex<Option<BufferPoolOptions>>,
    fail_pool: AtomicBool,
    fail_monitor: AtomicBool,
    unsupported_kind: AtomicBool,
}

impl BufferPooling for Scheduler {
    type Pool = TestPool;
    fn buffer_pool(&self, options: BufferPoolOptions) -> io::Result<TestPool> {
        self.pools.fetch_add(1, Ordering::Relaxed);
        *self.options.lock().unwrap() = Some(options);
        if self.fail_pool.load(Ordering::Relaxed) {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        Ok(TestPool(HeapBufferPool::new(
            options.buffers,
            options.buffer_size,
        )?))
    }
}
impl SocketOperation for Scheduler {
    type Socket = Arc<Socket>;
}
impl SocketRecvPoll for Scheduler {
    async fn socket_recv_poll(
        &self,
        _: &Arc<Socket>,
        _: &mut [u8],
        _: SocketRecvFlags,
        _: &Cancellation,
    ) -> OperationResult<usize> {
        unreachable!("packet reads use message operations")
    }
}
impl SocketSendPoll for Scheduler {
    async fn socket_send_poll(
        &self,
        _: &Arc<Socket>,
        _: &[u8],
        _: SocketSendFlags,
        _: &Cancellation,
    ) -> OperationResult<usize> {
        unreachable!("packet writes use message operations")
    }
}
impl SocketRecvMsgPoll for Scheduler {
    async fn socket_recvmsg_poll(
        &self,
        socket: &Arc<Socket>,
        buffers: &mut [IoSliceMut<'_>],
        _: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<SocketRecvResult> {
        cancellation.check()?;
        if let Some(error) = socket.error.lock().unwrap().take() {
            return Err(io::Error::from(error).into());
        }
        let packet = socket.incoming.lock().unwrap().pop_front();
        if let Some(packet) = packet {
            let mut bytes = 0;
            for buffer in buffers {
                let count = buffer.len().min(packet.bytes.len() - bytes);
                buffer[..count].copy_from_slice(&packet.bytes[bytes..bytes + count]);
                bytes += count;
            }
            return Ok(SocketRecvResult {
                bytes,
                address: packet.address,
                truncated: bytes < packet.bytes.len(),
                control_truncated: packet.control_truncated,
            });
        }
        cancellation.cancelled().await;
        Err(Cancelled.into())
    }
}
impl SocketSendMsgPoll for Scheduler {
    async fn socket_sendmsg_poll(
        &self,
        socket: &Arc<Socket>,
        buffers: &[IoSlice<'_>],
        address: Option<SocketAddress>,
        _: SocketSendFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        cancellation.check()?;
        if socket.pending_send.load(Ordering::Relaxed) {
            cancellation.cancelled().await;
            return Err(Cancelled.into());
        }
        if let Some(error) = socket.error.lock().unwrap().take() {
            return Err(io::Error::from(error).into());
        }
        socket.sends.fetch_add(1, Ordering::Relaxed);
        let mut sent = socket.send.lock().unwrap();
        sent.1 = 0;
        sent.2 = address;
        for buffer in buffers {
            let start = sent.1;
            sent.0[start..start + buffer.len()].copy_from_slice(buffer);
            sent.1 += buffer.len();
        }
        Ok(if socket.partial_send.load(Ordering::Relaxed) {
            sent.1 / 2
        } else {
            sent.1
        })
    }
}
impl SocketWait for Scheduler {
    async fn socket_wait(
        &self,
        socket: &Arc<Socket>,
        _: Interest,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        cancellation.check()?;
        socket.waits.fetch_add(1, Ordering::Relaxed);
        if socket.receive_on_wait.swap(false, Ordering::Relaxed) {
            socket.packet(b"ready");
            return Ok(());
        }
        if !socket.incoming.lock().unwrap().is_empty() {
            return Ok(());
        }
        cancellation.cancelled().await;
        Err(Cancelled.into())
    }
}
impl SocketMonitoring for Scheduler {
    type MonitorRegistration = ();
    fn socket_type(&self, socket: &Arc<Socket>) -> io::Result<SocketType> {
        if self.unsupported_kind.load(Ordering::Relaxed) {
            return Err(io::ErrorKind::Unsupported.into());
        }
        Ok(socket.kind)
    }
    fn socket_monitor_state(
        &self,
        socket: &Arc<Socket>,
        interest: Interest,
    ) -> io::Result<SocketMonitor<Arc<Socket>, ()>> {
        if self.fail_monitor.load(Ordering::Relaxed) {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        Ok(SocketMonitor::new(Arc::clone(socket), interest, ()))
    }
    async fn socket_monitor_close<M: SocketMonitorState<Arc<Socket>, ()>>(
        &self,
        monitor: &mut MonitorStorage<M>,
    ) -> io::Result<()> {
        let socket = monitor.as_ref().socket();
        if socket.pending_close.load(Ordering::Relaxed) {
            return pending().await;
        }
        if socket.fail_close.load(Ordering::Relaxed) {
            return Err(io::ErrorKind::Other.into());
        }
        Ok(())
    }
}

#[test]
fn packet_handles_validate_socket_kind_and_monitor_creation() {
    let scheduler = Scheduler::default();
    let stream = Socket::new(SocketType::Stream);
    assert!(
        matches!(scheduler.socket_packet_reader(&stream), Err(error) if error.kind() == io::ErrorKind::InvalidInput)
    );
    assert!(
        matches!(scheduler.socket_packet_writer(&stream), Err(error) if error.kind() == io::ErrorKind::InvalidInput)
    );
    let socket = Socket::new(SocketType::Datagram);
    scheduler.unsupported_kind.store(true, Ordering::Relaxed);
    assert!(
        matches!(scheduler.socket_packet_reader(&socket), Err(error) if error.kind() == io::ErrorKind::Unsupported)
    );
    assert!(
        matches!(scheduler.socket_packet_writer(&socket), Err(error) if error.kind() == io::ErrorKind::Unsupported)
    );
    scheduler.unsupported_kind.store(false, Ordering::Relaxed);
    scheduler.fail_monitor.store(true, Ordering::Relaxed);
    assert!(scheduler.socket_packet_reader(&socket).is_err());
    assert!(scheduler.socket_packet_writer(&socket).is_err());
    scheduler.fail_monitor.store(false, Ordering::Relaxed);
    for kind in [SocketType::Raw, SocketType::Other(123)] {
        let socket = Socket::new(kind);
        assert!(scheduler.socket_packet_reader(&socket).is_ok());
        assert!(scheduler.socket_packet_writer(&socket).is_ok());
    }
}

#[test]
fn borrowed_reads_reuse_storage_preserve_metadata_and_never_create_a_pool() {
    block_on(async {
        let scheduler = Scheduler::default();
        let socket = Socket::new(SocketType::Datagram);
        let mut reader = scheduler.socket_packet_reader(&socket).unwrap();
        let never = Cancellation::never();
        socket.packet(b"abcdef");
        // Initialize the mock's platform mutex before measuring operation allocations.
        drop(socket.error.lock().unwrap());
        let mut first = [0; 2];
        let mut second = [0; 2];
        let counts = allocation_counter::measure(|| {
            let mut vectors = [IoSliceMut::new(&mut first), IoSliceMut::new(&mut second)];
            let mut read = pin!(reader.read_vectored(&mut vectors, &never));
            let Poll::Ready(Ok(result)) =
                read.as_mut().poll(&mut Context::from_waker(Waker::noop()))
            else {
                panic!("queued packet must complete synchronously");
            };
            assert_eq!(result.bytes, 4);
            assert!(result.truncated);
            assert!(result.control_truncated);
            assert_eq!(result.address, Some("127.0.0.1:4567".parse().unwrap()));
        });
        assert_eq!(counts.count_total, 0);
        assert_eq!((&first, &second), (b"ab", b"cd"));
        socket.packet(b"");
        assert_eq!(reader.read_into(&mut first, &never).await.unwrap().bytes, 0);
        assert_eq!(scheduler.pools.load(Ordering::Relaxed), 0);
        reader.close().await.unwrap();
        assert_eq!(
            reader
                .read_into(&mut first, &never)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
        assert!(reader.read(&never).await.unwrap().is_none());
    });
}

#[test]
fn owned_reads_allocate_default_storage_lazily_and_keep_empty_packets() {
    block_on(async {
        let scheduler = Scheduler::default();
        let socket = Socket::new(SocketType::Datagram);
        let mut reader = scheduler.socket_packet_reader(&socket).unwrap();
        let cancelled = Cancellation::new();
        cancelled.cancel();
        assert!(matches!(
            reader.read_vectored(&mut [], &cancelled).await,
            Err(OperationError::Cancelled(_))
        ));
        assert!(matches!(
            reader.read(&cancelled).await,
            Err(OperationError::Cancelled(_))
        ));
        assert_eq!(scheduler.pools.load(Ordering::Relaxed), 0);
        socket.packet(b"");
        let empty = reader.read(&Cancellation::never()).await.unwrap().unwrap();
        assert!(empty.payload().is_empty());
        assert_eq!(
            *scheduler.options.lock().unwrap(),
            Some(BufferPoolOptions::new(2, 64 * 1024))
        );
        socket.packet(b"next");
        let packet = reader.read(&Cancellation::never()).await.unwrap().unwrap();
        assert_eq!(packet.payload(), b"next");
        assert_eq!(scheduler.pools.load(Ordering::Relaxed), 1);
        reader.close().await.unwrap();
        assert_eq!(packet.payload(), b"next");
    });
}

#[test]
fn custom_pools_backpressure_and_outstanding_packets_survive_replacement() {
    block_on(async {
        let scheduler = Scheduler::default();
        let socket = Socket::new(SocketType::Datagram);
        let old_pool = TestPool(HeapBufferPool::new(1, 4).unwrap());
        let mut reader = BoundSocketPacketReader::new(&scheduler, &socket)
            .unwrap()
            .with_pool(old_pool.clone());
        socket.packet(b"first");
        let old = reader.read(&Cancellation::never()).await.unwrap().unwrap();
        assert_eq!(old.payload(), b"firs");
        assert!(old.truncated());
        let cancellation = Cancellation::new();
        {
            let mut reading = pin!(reader.read(&cancellation));
            assert!(poll_once(reading.as_mut()).await.is_none());
            cancellation.cancel();
            assert!(matches!(reading.await, Err(OperationError::Cancelled(_))));
        }
        let new_pool = TestPool(HeapBufferPool::new(1, 8).unwrap());
        let mut reader = reader.with_pool(new_pool.clone());
        socket.packet(b"second");
        let next = reader.read(&Cancellation::never()).await.unwrap().unwrap();
        assert_eq!(next.payload(), b"second");
        assert!(old_pool.try_acquire().is_none());
        assert!(new_pool.try_acquire().is_none());
        drop(old);
        assert!(old_pool.try_acquire().is_some());
        assert_eq!(scheduler.pools.load(Ordering::Relaxed), 0);
        drop(next);
    });
}

#[test]
fn idle_owned_reads_return_the_lease_before_waiting_and_survive_cancellation() {
    block_on(async {
        let scheduler = Scheduler::default();
        let socket = Socket::new(SocketType::Datagram);
        let pool = TestPool(HeapBufferPool::new(1, 8).unwrap());
        let mut reader = scheduler
            .socket_packet_reader_with_pool(&socket, pool.clone())
            .unwrap();
        let cancellation = Cancellation::new();
        {
            let mut reading = pin!(reader.read(&cancellation));
            assert!(poll_once(reading.as_mut()).await.is_none());
            assert_eq!(socket.waits.load(Ordering::Relaxed), 1);
            assert!(pool.try_acquire().is_some());
            cancellation.cancel();
            assert!(matches!(reading.await, Err(OperationError::Cancelled(_))));
        }
        {
            let never = Cancellation::never();
            let mut reading = pin!(reader.read(&never));
            assert!(poll_once(reading.as_mut()).await.is_none());
        }
        assert!(pool.try_acquire().is_some());
        socket.receive_on_wait.store(true, Ordering::Relaxed);
        assert_eq!(
            reader
                .read(&Cancellation::never())
                .await
                .unwrap()
                .unwrap()
                .payload(),
            b"ready"
        );
    });
}

#[test]
fn pool_and_receive_failures_leave_packet_reader_usable() {
    block_on(async {
        let scheduler = Scheduler::default();
        let socket = Socket::new(SocketType::Datagram);
        let mut reader = scheduler.socket_packet_reader(&socket).unwrap();
        scheduler.fail_pool.store(true, Ordering::Relaxed);
        assert_eq!(
            reader
                .read(&Cancellation::never())
                .await
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::OutOfMemory
        );
        scheduler.fail_pool.store(false, Ordering::Relaxed);
        *socket.error.lock().unwrap() = Some(io::ErrorKind::ConnectionRefused);
        assert_eq!(
            reader
                .read(&Cancellation::never())
                .await
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::ConnectionRefused
        );
        socket.packet(b"retry");
        assert_eq!(
            reader
                .read(&Cancellation::never())
                .await
                .unwrap()
                .unwrap()
                .payload(),
            b"retry"
        );
        *socket.error.lock().unwrap() = Some(io::ErrorKind::ConnectionRefused);
        assert_eq!(
            reader
                .read_into(&mut [0; 8], &Cancellation::never())
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::ConnectionRefused
        );
        socket.packet(b"again");
        assert_eq!(
            reader
                .read_into(&mut [0; 8], &Cancellation::never())
                .await
                .unwrap()
                .bytes,
            5
        );
    });
}

#[test]
fn packet_writes_preserve_one_send_and_owned_storage_on_every_result() {
    block_on(async {
        let scheduler = Scheduler::default();
        let socket = Socket::new(SocketType::Datagram);
        let mut writer = scheduler.socket_packet_writer(&socket).unwrap();
        let never = Cancellation::never();
        let address = Some("127.0.0.1:1234".parse().unwrap());
        // Initialize the mock's platform mutexes outside the I/O allocation measurement.
        drop(socket.error.lock().unwrap());
        drop(socket.send.lock().unwrap());
        let counts = allocation_counter::measure(|| {
            assert_eq!(
                {
                    let vectors = [IoSlice::new(b"ab"), IoSlice::new(b"cd")];
                    let mut write = pin!(writer.write_vectored(&vectors, address, &never));
                    let Poll::Ready(Ok(bytes)) =
                        write.as_mut().poll(&mut Context::from_waker(Waker::noop()))
                    else {
                        panic!("ready packet send must complete synchronously");
                    };
                    bytes
                },
                4
            );
        });
        assert_eq!(counts.count_total, 0);
        assert_eq!(&socket.send.lock().unwrap().0[..4], b"abcd");
        assert_eq!(socket.send.lock().unwrap().2, address);
        assert_eq!(writer.write(b"", None, &never).await.unwrap(), 0);
        socket.partial_send.store(true, Ordering::Relaxed);
        assert_eq!(writer.write(b"1234", None, &never).await.unwrap(), 2);
        assert_eq!(socket.sends.load(Ordering::Relaxed), 3);
        let buffer = b"owned".to_vec();
        let pointer = buffer.as_ptr();
        let (result, buffer) = writer.write_owned(buffer, address, &never).await;
        assert_eq!(result.unwrap(), 2);
        assert_eq!(buffer.as_ptr(), pointer);
        *socket.error.lock().unwrap() = Some(io::ErrorKind::ConnectionRefused);
        let (result, buffer) = writer.write_owned(buffer, None, &never).await;
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::ConnectionRefused);
        assert_eq!(buffer.as_ptr(), pointer);
        let cancellation = Cancellation::new();
        cancellation.cancel();
        let (result, buffer) = writer.write_owned(buffer, None, &cancellation).await;
        assert!(matches!(result, Err(OperationError::Cancelled(_))));
        assert_eq!(buffer.as_ptr(), pointer);
        socket.pending_send.store(true, Ordering::Relaxed);
        let cancellation = Cancellation::new();
        let (result, buffer) = {
            let mut write = pin!(writer.write_owned(buffer, None, &cancellation));
            assert!(poll_once(write.as_mut()).await.is_none());
            cancellation.cancel();
            write.await
        };
        assert!(matches!(result, Err(OperationError::Cancelled(_))));
        assert_eq!(buffer.as_ptr(), pointer);
        socket.pending_send.store(false, Ordering::Relaxed);
        assert!(writer.write(b"ok", None, &never).await.is_ok());
    });
}

#[test]
fn abandoned_and_failed_close_prevent_further_packet_io_but_allow_cleanup_retries() {
    block_on(async {
        let scheduler = Scheduler::default();
        let socket = Socket::new(SocketType::Datagram);
        let mut reader = scheduler.socket_packet_reader(&socket).unwrap();
        let mut writer = scheduler.socket_packet_writer(&socket).unwrap();
        socket.pending_close.store(true, Ordering::Relaxed);
        assert!(poll_once(reader.close()).await.is_none());
        assert!(poll_once(writer.close()).await.is_none());
        let never = Cancellation::never();
        assert!(reader.read(&never).await.unwrap().is_none());
        assert_eq!(
            reader.read_into(&mut [], &never).await.unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(
            writer.write(b"x", None, &never).await.unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        let (result, returned) = writer.write_owned(vec![1], None, &never).await;
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(returned, [1]);
        socket.pending_close.store(false, Ordering::Relaxed);
        socket.fail_close.store(true, Ordering::Relaxed);
        assert!(reader.close().await.is_err());
        assert!(writer.close().await.is_err());
        socket.fail_close.store(false, Ordering::Relaxed);
        reader.close().await.unwrap();
        writer.close().await.unwrap();
        reader.close().await.unwrap();
        writer.close().await.unwrap();
    });
}

impl SocketPacketReading for Scheduler {}
impl SocketPacketWriting for Scheduler {}

#[test]
fn opaque_packet_handles_retain_the_resource_without_borrowing_its_input_reference() {
    block_on(async {
        let scheduler = Scheduler::default();
        let mut reader = {
            let socket = Socket::new(SocketType::Datagram);
            socket.packet(b"retained");
            scheduler.socket_packet_reader(&socket).unwrap()
        };
        assert_eq!(
            reader
                .read(&Cancellation::never())
                .await
                .unwrap()
                .unwrap()
                .payload(),
            b"retained"
        );
        reader.close().await.unwrap();
        let mut writer = {
            let socket = Socket::new(SocketType::Datagram);
            scheduler.socket_packet_writer(&socket).unwrap()
        };
        assert_eq!(
            writer
                .write(b"retained", None, &Cancellation::never())
                .await
                .unwrap(),
            8
        );
        writer.close().await.unwrap();
    });
}

// A backend can retain native metadata and replace the monitor composition without changing protocol code.
struct NativeAddress {
    port: u16,
    scope: u32,
}
impl SocketAddressView for NativeAddress {
    fn to_socket_address(&self) -> SocketAddress {
        std::net::SocketAddrV6::new(std::net::Ipv6Addr::LOCALHOST, self.port, 7, self.scope).into()
    }
}
struct NativeMessage {
    address: NativeAddress,
    payload: <TestPool as BufferPool>::Buffer,
}
impl SocketRecvMessage for NativeMessage {
    type Payload = <TestPool as BufferPool>::Buffer;
    fn address(&self) -> Option<impl SocketAddressView + '_> {
        Some(&self.address)
    }
    fn payload(&self) -> &[u8] {
        self.payload.as_ref()
    }
    fn truncated(&self) -> bool {
        false
    }
    fn control_truncated(&self) -> bool {
        false
    }
    fn into_payload(self) -> Self::Payload {
        self.payload
    }
}
impl SocketRecvMessageMut for NativeMessage {
    fn payload_mut(&mut self) -> &mut [u8] {
        self.payload.as_mut()
    }
}
struct Specialized(Scheduler);
impl SocketOperation for Specialized {
    type Socket = Arc<Socket>;
}
impl BufferPooling for Specialized {
    type Pool = TestPool;
    fn buffer_pool(&self, options: BufferPoolOptions) -> io::Result<TestPool> {
        self.0.buffer_pool(options)
    }
}
impl SocketRecvPoll for Specialized {
    async fn socket_recv_poll(
        &self,
        _: &Arc<Socket>,
        _: &mut [u8],
        _: SocketRecvFlags,
        _: &Cancellation,
    ) -> OperationResult<usize> {
        panic!("specialized reader must bypass raw polling")
    }
}
impl SocketSendPoll for Specialized {
    async fn socket_send_poll(
        &self,
        _: &Arc<Socket>,
        _: &[u8],
        _: SocketSendFlags,
        _: &Cancellation,
    ) -> OperationResult<usize> {
        panic!("specialized reader does not send")
    }
}
impl SocketRecvMsgPoll for Specialized {
    async fn socket_recvmsg_poll(
        &self,
        _: &Arc<Socket>,
        _: &mut [IoSliceMut<'_>],
        _: SocketRecvFlags,
        _: &Cancellation,
    ) -> OperationResult<SocketRecvResult> {
        panic!("specialized reader must bypass raw polling")
    }
}
impl SocketSendMsgPoll for Specialized {
    async fn socket_sendmsg_poll(
        &self,
        _: &Arc<Socket>,
        _: &[IoSlice<'_>],
        _: Option<SocketAddress>,
        _: SocketSendFlags,
        _: &Cancellation,
    ) -> OperationResult<usize> {
        panic!("specialized reader does not send")
    }
}
impl SocketWait for Specialized {
    async fn socket_wait(
        &self,
        _: &Arc<Socket>,
        _: Interest,
        _: &Cancellation,
    ) -> OperationResult<()> {
        panic!("specialized reader must bypass raw readiness")
    }
}
impl SocketMonitoring for Specialized {
    type MonitorRegistration = ();
    fn socket_type(&self, _: &Arc<Socket>) -> io::Result<SocketType> {
        panic!("specialized factory validates its own resource")
    }
    fn socket_monitor_state(
        &self,
        _: &Arc<Socket>,
        _: Interest,
    ) -> io::Result<SocketMonitor<Arc<Socket>, ()>> {
        panic!("specialized factory must not construct a monitor")
    }
}
struct NativeReader<'a> {
    scheduler: &'a Specialized,
    pool: Option<TestPool>,
    closed: bool,
}
impl SocketPacketReader for NativeReader<'_> {
    type Message = NativeMessage;
    async fn read(
        &mut self,
        cancellation: &Cancellation,
    ) -> OperationResult<Option<NativeMessage>> {
        cancellation.check()?;
        if self.closed {
            return Ok(None);
        }
        if self.pool.is_none() {
            self.pool = Some(self.scheduler.buffer_pool(BufferPoolOptions::new(1, 16))?);
        }
        let mut payload = self.pool.as_ref().unwrap().acquire(cancellation).await?;
        payload.as_mut()[..6].copy_from_slice(b"native");
        payload.truncate(6);
        Ok(Some(NativeMessage {
            address: NativeAddress {
                port: 4567,
                scope: 9,
            },
            payload,
        }))
    }
    async fn read_into(
        &mut self,
        buffer: &mut [u8],
        cancellation: &Cancellation,
    ) -> OperationResult<SocketRecvResult> {
        self.read_vectored(&mut [IoSliceMut::new(buffer)], cancellation)
            .await
    }
    async fn read_vectored(
        &mut self,
        buffers: &mut [IoSliceMut<'_>],
        cancellation: &Cancellation,
    ) -> OperationResult<SocketRecvResult> {
        cancellation.check()?;
        let mut bytes = 0;
        for buffer in buffers {
            let count = buffer.len().min(6 - bytes);
            buffer[..count].copy_from_slice(&b"native"[bytes..bytes + count]);
            bytes += count;
        }
        Ok(SocketRecvResult {
            bytes,
            address: Some(
                NativeAddress {
                    port: 4567,
                    scope: 9,
                }
                .to_socket_address(),
            ),
            truncated: bytes < 6,
            control_truncated: false,
        })
    }
    async fn close(&mut self) -> io::Result<()> {
        self.closed = true;
        // This borrow makes the scheduler lifetime part of the replacement handle as well.
        assert_eq!(self.scheduler.0.pools.load(Ordering::Relaxed), 1);
        Ok(())
    }
}
impl SocketPacketReading for Specialized {
    fn socket_packet_reader<'scheduler>(
        &'scheduler self,
        socket: &Self::Socket,
    ) -> io::Result<
        impl SocketPacketReader<
            Message: SocketRecvMessageMut<Payload = <Self::Pool as BufferPool>::Buffer>,
        > + 'scheduler,
    > {
        assert_eq!(socket.kind, SocketType::Datagram);
        Ok(NativeReader {
            scheduler: self,
            pool: None,
            closed: false,
        })
    }
    fn socket_packet_reader_with_pool<'scheduler>(
        &'scheduler self,
        socket: &Self::Socket,
        pool: Self::Pool,
    ) -> io::Result<
        impl SocketPacketReader<
            Message: SocketRecvMessageMut<Payload = <Self::Pool as BufferPool>::Buffer>,
        > + 'scheduler,
    > {
        assert_eq!(socket.kind, SocketType::Datagram);
        Ok(NativeReader {
            scheduler: self,
            pool: Some(pool),
            closed: false,
        })
    }
}

#[test]
fn packet_factory_specialization_preserves_native_metadata_and_owned_payload() {
    block_on(async {
        let scheduler = Specialized(Scheduler::default());
        let socket = Socket::new(SocketType::Datagram);
        let never = Cancellation::never();
        let mut reader = scheduler.socket_packet_reader(&socket).unwrap();
        assert_eq!(scheduler.0.pools.load(Ordering::Relaxed), 0);
        let mut message = reader.read(&never).await.unwrap().unwrap();
        assert_eq!(message.payload(), b"native");
        let endpoint = message.address().unwrap().to_socket_address();
        assert_eq!(endpoint.port(), 4567);
        assert!(
            matches!(endpoint, SocketAddress::V6(address) if address.scope_id() == 9 && address.flowinfo() == 7)
        );
        message.payload_mut()[0] = b'N';
        let mut held = message.into_payload().freeze();
        assert_eq!(held.as_ref(), b"Native");
        reader.close().await.unwrap();
        assert!(reader.read(&never).await.unwrap().is_none());
        drop(reader);
        held = std::thread::spawn(move || {
            assert_eq!(held.as_ref(), b"Native");
            held
        })
        .join()
        .unwrap();
        drop(held);
        let pool = TestPool(HeapBufferPool::new(1, 16).unwrap());
        let mut reader = scheduler
            .socket_packet_reader_with_pool(&socket, pool.clone())
            .unwrap();
        let mut bytes = [0; 6];
        assert_eq!(reader.read_into(&mut bytes, &never).await.unwrap().bytes, 6);
        assert_eq!(&bytes, b"native");
        assert_eq!(
            reader
                .read_vectored(&mut [IoSliceMut::new(&mut bytes[..2])], &never)
                .await
                .unwrap()
                .bytes,
            2
        );
        let message = reader.read(&never).await.unwrap().unwrap();
        reader.close().await.unwrap();
        assert!(pool.try_acquire().is_none());
        drop(message);
        assert!(pool.try_acquire().is_some());
    });
}
