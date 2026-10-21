// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{NativeScheduler, Selector};
use core_scheduler::*;
use std::{
    io,
    io::{IoSlice, IoSliceMut, SeekFrom},
    net::{Shutdown, TcpListener, TcpStream},
    path::Path,
};

impl<S: Selector + FileOperation> FileOperation for NativeScheduler<S> {
    type File = S::File;
}

impl<S: Selector + FileOpen> FileOpen for NativeScheduler<S> {
    async fn file_open(
        &self,
        path: &Path,
        options: FileOptions,
        cancellation: &Cancellation,
    ) -> OperationResult<Self::File> {
        self.selector().file_open(path, options, cancellation).await
    }
}

impl<S: Selector + FileClose> FileClose for NativeScheduler<S> {
    async fn file_close(&self, file: Self::File) -> std::io::Result<()> {
        self.selector().file_close(file).await
    }
    async fn file_close_submit(&self, file: Self::File) -> std::io::Result<()> {
        self.selector().file_close_submit(file).await
    }
}

impl<S: Selector + FileRead> FileRead for NativeScheduler<S> {
    async fn file_read<B: IoBufMut>(
        &self,
        file: &Self::File,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.selector().file_read(file, buffer, cancellation).await
    }
}

impl<S: Selector + FileWrite> FileWrite for NativeScheduler<S> {
    async fn file_write<B: IoBuf>(
        &self,
        file: &Self::File,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.selector().file_write(file, buffer, cancellation).await
    }
}

impl<S: Selector + FileReadAt> FileReadAt for NativeScheduler<S> {
    async fn file_read_at<B: IoBufMut>(
        &self,
        file: &Self::File,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.selector()
            .file_read_at(file, buffer, offset, cancellation)
            .await
    }
}

impl<S: Selector + FileWriteAt> FileWriteAt for NativeScheduler<S> {
    async fn file_write_at<B: IoBuf>(
        &self,
        file: &Self::File,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.selector()
            .file_write_at(file, buffer, offset, cancellation)
            .await
    }
}

impl<S: Selector + FileSeek> FileSeek for NativeScheduler<S> {
    async fn file_seek(
        &self,
        file: &Self::File,
        position: SeekFrom,
        cancellation: &Cancellation,
    ) -> OperationResult<u64> {
        self.selector()
            .file_seek(file, position, cancellation)
            .await
    }
}

impl<S: Selector + FileTruncate> FileTruncate for NativeScheduler<S> {
    async fn file_truncate(
        &self,
        file: &Self::File,
        length: u64,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        self.selector()
            .file_truncate(file, length, cancellation)
            .await
    }
}

impl<S: Selector + FileSync> FileSync for NativeScheduler<S> {
    async fn file_sync(
        &self,
        file: &Self::File,
        mode: SyncMode,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        self.selector().file_sync(file, mode, cancellation).await
    }
}

impl<S: Selector + FileStat> FileStat for NativeScheduler<S> {
    type Metadata = S::Metadata;
    async fn file_stat(
        &self,
        file: &Self::File,
        cancellation: &Cancellation,
    ) -> OperationResult<Self::Metadata> {
        self.selector().file_stat(file, cancellation).await
    }
}

impl<S: Selector + FileAllocate> FileAllocate for NativeScheduler<S> {
    async fn file_allocate(
        &self,
        file: &Self::File,
        offset: u64,
        length: u64,
        mode: AllocationMode,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        self.selector()
            .file_allocate(file, offset, length, mode, cancellation)
            .await
    }
}

impl<S: Selector + FileAdvice> FileAdvice for NativeScheduler<S> {
    async fn file_advise(
        &self,
        file: &Self::File,
        offset: u64,
        length: u64,
        advice: AdviceMode,
    ) -> io::Result<()> {
        self.selector()
            .file_advise(file, offset, length, advice)
            .await
    }
    async fn file_advise_submit(
        &self,
        file: &Self::File,
        offset: u64,
        length: u64,
        advice: AdviceMode,
    ) -> io::Result<()> {
        self.selector()
            .file_advise_submit(file, offset, length, advice)
            .await
    }
}

impl<S: Selector + SocketOperation> SocketOperation for NativeScheduler<S> {
    type Socket = S::Socket;
}

impl<S: Selector + SocketOpen> SocketOpen for NativeScheduler<S> {
    async fn socket_open(
        &self,
        domain: SocketDomain,
        kind: SocketType,
        protocol: Option<SocketProtocol>,
        cancellation: &Cancellation,
    ) -> OperationResult<Self::Socket> {
        self.selector()
            .socket_open(domain, kind, protocol, cancellation)
            .await
    }
}

impl<S: Selector + SocketBind> SocketBind for NativeScheduler<S> {
    async fn socket_bind(
        &self,
        socket: &Self::Socket,
        address: SocketAddress,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        self.selector()
            .socket_bind(socket, address, cancellation)
            .await
    }
}

impl<S: Selector + SocketListen> SocketListen for NativeScheduler<S> {
    async fn socket_listen(
        &self,
        socket: &Self::Socket,
        backlog: i32,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        self.selector()
            .socket_listen(socket, backlog, cancellation)
            .await
    }
}

impl<S: Selector + SocketConnect> SocketConnect for NativeScheduler<S> {
    async fn socket_connect(
        &self,
        socket: &Self::Socket,
        address: SocketAddress,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        self.selector()
            .socket_connect(socket, address, cancellation)
            .await
    }
}

impl<S: Selector + SocketAccept> SocketAccept for NativeScheduler<S> {
    async fn socket_accept(
        &self,
        socket: &Self::Socket,
        cancellation: &Cancellation,
    ) -> OperationResult<(Self::Socket, SocketAddress)> {
        self.selector().socket_accept(socket, cancellation).await
    }
}

impl<S: Selector + SocketLocalAddress> SocketLocalAddress for NativeScheduler<S> {
    fn socket_local_address(&self, socket: &Self::Socket) -> io::Result<SocketAddress> {
        self.selector().socket_local_address(socket)
    }
}

impl<S: Selector + SocketPeerAddress> SocketPeerAddress for NativeScheduler<S> {
    fn socket_peer_address(&self, socket: &Self::Socket) -> io::Result<SocketAddress> {
        self.selector().socket_peer_address(socket)
    }
}

impl<S: Selector + SocketRead> SocketRead for NativeScheduler<S> {
    async fn socket_read<B: IoBufMut>(
        &self,
        socket: &Self::Socket,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.selector()
            .socket_read(socket, buffer, cancellation)
            .await
    }
}

impl<S: Selector + SocketWrite> SocketWrite for NativeScheduler<S> {
    async fn socket_write<B: IoBuf>(
        &self,
        socket: &Self::Socket,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.selector()
            .socket_write(socket, buffer, cancellation)
            .await
    }
}

impl<S: Selector + SocketWait> SocketWait for NativeScheduler<S> {
    async fn socket_wait(
        &self,
        socket: &Self::Socket,
        interest: Interest,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        self.selector()
            .socket_wait(socket, interest, cancellation)
            .await
    }
}

impl<S: Selector + SocketClose> SocketClose for NativeScheduler<S> {
    async fn socket_close(&self, socket: Self::Socket) -> io::Result<()> {
        self.selector().socket_close(socket).await
    }
    async fn socket_close_submit(&self, socket: Self::Socket) -> io::Result<()> {
        self.selector().socket_close_submit(socket).await
    }
}

impl<S: Selector + SocketShutdown> SocketShutdown for NativeScheduler<S> {
    async fn socket_shutdown(&self, socket: &Self::Socket, how: Shutdown) -> io::Result<()> {
        self.selector().socket_shutdown(socket, how).await
    }
    async fn socket_shutdown_submit(&self, socket: &Self::Socket, how: Shutdown) -> io::Result<()> {
        self.selector().socket_shutdown_submit(socket, how).await
    }
}

impl<S: Selector + SocketSendMsg> SocketSendMsg for NativeScheduler<S> {
    async fn socket_sendmsg<Buffers: IoBufSequence>(
        &self,
        socket: &Self::Socket,
        message: SocketMessage<Buffers>,
        flags: SocketSendFlags,
        cancellation: &Cancellation,
    ) -> (OperationResult<usize>, SocketMessage<Buffers>) {
        self.selector()
            .socket_sendmsg(socket, message, flags, cancellation)
            .await
    }
}

impl<S: Selector + SocketRecvMsg> SocketRecvMsg for NativeScheduler<S> {
    async fn socket_recvmsg<Buffers: IoBufSequenceMut>(
        &self,
        socket: &Self::Socket,
        buffers: Buffers,
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> (OperationResult<SocketRecvResult>, Buffers) {
        self.selector()
            .socket_recvmsg(socket, buffers, flags, cancellation)
            .await
    }
}

impl<S: Selector + SocketRecvPoll> SocketRecvPoll for NativeScheduler<S> {
    async fn socket_recv_poll(
        &self,
        socket: &Self::Socket,
        buffer: &mut [u8],
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.selector()
            .socket_recv_poll(socket, buffer, flags, cancellation)
            .await
    }
}

impl<S: Selector + SocketSendPoll> SocketSendPoll for NativeScheduler<S> {
    async fn socket_send_poll(
        &self,
        socket: &Self::Socket,
        buffer: &[u8],
        flags: SocketSendFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.selector()
            .socket_send_poll(socket, buffer, flags, cancellation)
            .await
    }
}

impl<S: Selector + SocketRecvMsgPoll> SocketRecvMsgPoll for NativeScheduler<S> {
    async fn socket_recvmsg_poll(
        &self,
        socket: &Self::Socket,
        buffers: &mut [IoSliceMut<'_>],
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<SocketRecvResult> {
        self.selector()
            .socket_recvmsg_poll(socket, buffers, flags, cancellation)
            .await
    }
}

/// A receive resource advanced through the Native scheduler's explicit selector.
pub struct NativeRecvMsgReceiver<R>(R);

impl<S: Selector + SocketRecvMsgMultishot> SocketRecvMsgMultishot for NativeScheduler<S> {
    type Receiver = NativeRecvMsgReceiver<S::Receiver>;

    async fn socket_recvmsg_multishot(
        &self,
        socket: &Self::Socket,
        options: RecvMsgMultishotOptions,
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<Self::Receiver> {
        self.selector()
            .socket_recvmsg_multishot(socket, options, flags, cancellation)
            .await
            .map(NativeRecvMsgReceiver)
    }
}

impl<S, R> SocketRecvMsgReceiver<NativeScheduler<S>> for NativeRecvMsgReceiver<R>
where
    S: Selector,
    R: SocketRecvMsgReceiver<S>,
{
    type Buffer = R::Buffer;

    async fn recv(
        &mut self,
        scheduler: &NativeScheduler<S>,
        cancellation: &Cancellation,
    ) -> OperationResult<Option<RecvMsgMultishotMessage<Self::Buffer>>> {
        self.0.recv(scheduler.selector(), cancellation).await
    }

    async fn close(&mut self, scheduler: &NativeScheduler<S>) -> io::Result<()> {
        self.0.close(scheduler.selector()).await
    }
}

impl<S: Selector + SocketSendMsgPoll> SocketSendMsgPoll for NativeScheduler<S> {
    async fn socket_sendmsg_poll(
        &self,
        socket: &Self::Socket,
        buffers: &[IoSlice<'_>],
        address: Option<SocketAddress>,
        flags: SocketSendFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.selector()
            .socket_sendmsg_poll(socket, buffers, address, flags, cancellation)
            .await
    }
}

impl<S: Selector + SocketRegistration> SocketRegistration for NativeScheduler<S> {
    fn register_socket(&self, socket: TcpStream) -> io::Result<Self::Socket> {
        self.selector().register_socket(socket)
    }
    fn register_listener(&self, listener: TcpListener) -> io::Result<Self::Socket> {
        self.selector().register_listener(listener)
    }
}

impl<S: Selector + SocketGetOption<O>, O: SocketOption> SocketGetOption<O> for NativeScheduler<S> {
    async fn socket_get_option(
        &self,
        socket: &Self::Socket,
        option: O,
        cancellation: &Cancellation,
    ) -> OperationResult<O::Value> {
        self.selector()
            .socket_get_option(socket, option, cancellation)
            .await
    }
}

impl<S: Selector + SocketSetOption<O>, O: SocketOption> SocketSetOption<O> for NativeScheduler<S> {
    async fn socket_set_option(
        &self,
        socket: &Self::Socket,
        option: O,
        value: O::Value,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        self.selector()
            .socket_set_option(socket, option, value, cancellation)
            .await
    }
}

impl<S: Selector + AddressResolution> AddressResolution for NativeScheduler<S> {
    type Addresses = S::Addresses;
    async fn address_resolve(
        &self,
        name: Option<&str>,
        service: AddressService<'_>,
        hints: AddressHints,
        cancellation: &Cancellation,
    ) -> OperationResult<Self::Addresses> {
        self.selector()
            .address_resolve(name, service, hints, cancellation)
            .await
    }
}

impl<S: Selector + Blocking> Blocking for NativeScheduler<S> {
    type Handle<Output: Send + 'static> = S::Handle<Output>;
    async fn spawn_blocking<F, Output>(
        &self,
        operation: F,
        cancellation: &Cancellation,
    ) -> Result<Self::Handle<Output>, BlockingError>
    where
        F: FnOnce() -> Output + Send + 'static,
        Output: Send + 'static,
    {
        self.selector()
            .spawn_blocking(operation, cancellation)
            .await
    }
}

impl<S: Selector + Signals> Signals for NativeScheduler<S> {
    type Signal = S::Signal;
    type Subscription = S::Subscription;
    fn signal_subscribe<Target>(
        &self,
        signals: &[Self::Signal],
        target: Target,
    ) -> io::Result<Self::Subscription>
    where
        Target: Notify<Self::Signal> + 'static,
    {
        self.selector().signal_subscribe(signals, target)
    }
}

impl<S: Selector + SocketMonitoring> SocketMonitoring for NativeScheduler<S>
where
    S::Socket: 'static,
{
    type MonitorRegistration = S::MonitorRegistration;
    fn socket_monitor_state(
        &self,
        socket: &Self::Socket,
        interest: Interest,
    ) -> io::Result<SocketMonitor<Self::Socket, Self::MonitorRegistration>> {
        self.selector().socket_monitor_state(socket, interest)
    }
    fn socket_stream_monitor_state(
        &self,
        socket: &Self::Socket,
        interest: Interest,
    ) -> io::Result<SocketStreamMonitor<Self::Socket, Self::MonitorRegistration>> {
        self.selector()
            .socket_stream_monitor_state(socket, interest)
    }
    async fn socket_monitor_wait<M: SocketMonitorState<Self::Socket, Self::MonitorRegistration>>(
        &self,
        monitor: &mut MonitorStorage<M>,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        self.selector()
            .socket_monitor_wait(monitor, cancellation)
            .await
    }
    async fn socket_monitor_close<
        M: SocketMonitorState<Self::Socket, Self::MonitorRegistration>,
    >(
        &self,
        monitor: &mut MonitorStorage<M>,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        self.selector()
            .socket_monitor_close(monitor, cancellation)
            .await
    }
    async fn socket_monitor_recv_poll<
        M: SocketMonitorState<Self::Socket, Self::MonitorRegistration>,
    >(
        &self,
        monitor: &mut MonitorStorage<M>,
        buffer: &mut [u8],
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.selector()
            .socket_monitor_recv_poll(monitor, buffer, flags, cancellation)
            .await
    }
    async fn socket_monitor_send_poll<
        M: SocketMonitorState<Self::Socket, Self::MonitorRegistration>,
    >(
        &self,
        monitor: &mut MonitorStorage<M>,
        buffer: &[u8],
        flags: SocketSendFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.selector()
            .socket_monitor_send_poll(monitor, buffer, flags, cancellation)
            .await
    }
    async fn socket_monitor_recvmsg_poll<
        M: SocketMonitorState<Self::Socket, Self::MonitorRegistration>,
    >(
        &self,
        monitor: &mut MonitorStorage<M>,
        buffers: &mut [IoSliceMut<'_>],
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<SocketRecvResult> {
        self.selector()
            .socket_monitor_recvmsg_poll(monitor, buffers, flags, cancellation)
            .await
    }
    async fn socket_monitor_sendmsg_poll<
        M: SocketMonitorState<Self::Socket, Self::MonitorRegistration>,
    >(
        &self,
        monitor: &mut MonitorStorage<M>,
        buffers: &[IoSlice<'_>],
        address: Option<SocketAddress>,
        flags: SocketSendFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.selector()
            .socket_monitor_sendmsg_poll(monitor, buffers, address, flags, cancellation)
            .await
    }
}

impl<S: Selector + FileMonitoring> FileMonitoring for NativeScheduler<S> {
    type FileRegistration = S::FileRegistration;
    fn file_monitor(
        &self,
        file: &Self::File,
    ) -> io::Result<FileMonitor<Self::File, Self::FileRegistration>> {
        self.selector().file_monitor(file)
    }
    async fn file_monitor_read<B: IoBufMut>(
        &self,
        monitor: &mut FileMonitor<Self::File, Self::FileRegistration>,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.selector()
            .file_monitor_read(monitor, buffer, cancellation)
            .await
    }
    async fn file_monitor_write<B: IoBuf>(
        &self,
        monitor: &mut FileMonitor<Self::File, Self::FileRegistration>,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.selector()
            .file_monitor_write(monitor, buffer, cancellation)
            .await
    }
    async fn file_monitor_read_at<B: IoBufMut>(
        &self,
        monitor: &mut FileMonitor<Self::File, Self::FileRegistration>,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.selector()
            .file_monitor_read_at(monitor, buffer, offset, cancellation)
            .await
    }
    async fn file_monitor_write_at<B: IoBuf>(
        &self,
        monitor: &mut FileMonitor<Self::File, Self::FileRegistration>,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.selector()
            .file_monitor_write_at(monitor, buffer, offset, cancellation)
            .await
    }
    async fn file_monitor_close(
        &self,
        monitor: &mut FileMonitor<Self::File, Self::FileRegistration>,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        self.selector()
            .file_monitor_close(monitor, cancellation)
            .await
    }
}
