// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Keep the concrete io_uring control separate from the portable pooled receive benchmark.

use core_scheduler::*;

#[cfg(target_os = "linux")]
pub trait Receiver: Send {
    type Message: SocketRecvMessage;
    #[cfg(target_os = "linux")]
    fn recv(
        &mut self,
        cancellation: &Cancellation,
    ) -> impl std::future::Future<Output = OperationResult<Option<Self::Message>>> + Send;
    #[cfg(target_os = "linux")]
    fn close(&mut self) -> impl std::future::Future<Output = std::io::Result<()>> + Send;
}
#[cfg(target_os = "linux")]
impl Receiver for std::convert::Infallible {
    type Message = OwnedSocketRecvMessage<BufferLease>;
    #[cfg(target_os = "linux")]
    async fn recv(&mut self, _: &Cancellation) -> OperationResult<Option<Self::Message>> {
        match *self {}
    }
    #[cfg(target_os = "linux")]
    async fn close(&mut self) -> std::io::Result<()> {
        match *self {}
    }
}

pub trait Multishot: SocketOperation + BufferPooling + Sync {
    #[cfg(target_os = "linux")]
    type Receiver: Receiver;
    #[cfg(all(not(target_os = "linux"), test))]
    type Receiver: Send;

    #[cfg(any(target_os = "linux", test))]
    fn multishot(
        &self,
        _socket: &Self::Socket,
        _buffers: usize,
        _size: usize,
        _cancellation: &Cancellation,
    ) -> impl std::future::Future<Output = OperationResult<Self::Receiver>> + Send {
        async move {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "the multishot control requires io_uring; use pool for portable receives",
            )
            .into())
        }
    }
}

impl Multishot for core_scheduler_direct::DirectScheduler {
    #[cfg(any(target_os = "linux", test))]
    type Receiver = std::convert::Infallible;
}

impl Multishot for core_scheduler_tokio::TokioScheduler {
    #[cfg(any(target_os = "linux", test))]
    type Receiver = std::convert::Infallible;
}

impl Multishot for core_scheduler_native::NativeScheduler {
    #[cfg(any(target_os = "linux", test))]
    type Receiver = std::convert::Infallible;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_backends_reject_the_retired_multishot_control() {
        let scheduler = core_scheduler_direct::DirectScheduler::new(Default::default()).unwrap();
        futures_lite::future::block_on(async {
            let never = Cancellation::never();
            let socket = scheduler
                .socket_open(SocketDomain::Ipv4, SocketType::Datagram, None, &never)
                .await
                .unwrap();
            let error = scheduler
                .multishot(&socket, 8, 64, &never)
                .await
                .err()
                .unwrap();
            assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
            scheduler.socket_close(socket).await.unwrap();
            scheduler.shutdown().await;
        });
    }
}

#[cfg(target_os = "linux")]
mod iouring {
    use super::*;
    use core_scheduler_iouring::{IoUringSelector, RecvMsgMultishot, RecvMsgMultishotOptions};
    use core_scheduler_native::NativeScheduler;

    pub struct RingReceiver(RecvMsgMultishot);

    impl Receiver for RingReceiver {
        type Message =
            core_scheduler_iouring::IoUringRecvMessage<core_scheduler_iouring::RecvMsgBuffer>;

        async fn recv(
            &mut self,
            cancellation: &Cancellation,
        ) -> OperationResult<Option<Self::Message>> {
            self.0.recv(cancellation).await
        }

        async fn close(&mut self) -> std::io::Result<()> {
            self.0.close().await
        }
    }

    impl Multishot for NativeScheduler<IoUringSelector> {
        type Receiver = RingReceiver;

        async fn multishot(
            &self,
            socket: &Self::Socket,
            buffers: usize,
            size: usize,
            cancellation: &Cancellation,
        ) -> OperationResult<Self::Receiver> {
            self.selector()
                .socket_recvmsg_multishot(
                    socket,
                    RecvMsgMultishotOptions {
                        buffers: buffers.try_into().unwrap(),
                        buffer_size: size,
                    },
                    SocketRecvFlags::default(),
                    cancellation,
                )
                .await
                .map(RingReceiver)
        }
    }
}
