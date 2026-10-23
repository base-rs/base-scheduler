// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Keep the concrete io_uring control separate from the portable pooled receive benchmark.

use core_scheduler::*;

pub trait Multishot: SocketReceiving + Sync {
    type Receiver: SocketRecvMsgReceiver<Self>;

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
    type Receiver = <Self as SocketReceiving>::PooledReceiver;
}

impl Multishot for core_scheduler_tokio::TokioScheduler {
    type Receiver = <Self as SocketReceiving>::PooledReceiver;
}

impl Multishot for core_scheduler_native::NativeScheduler {
    type Receiver = <Self as SocketReceiving>::PooledReceiver;
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

    pub struct Receiver(RecvMsgMultishot);

    impl SocketRecvMsgReceiver<NativeScheduler<IoUringSelector>> for Receiver {
        type Message = <RecvMsgMultishot as SocketRecvMsgReceiver<IoUringSelector>>::Message;

        async fn recv(
            &mut self,
            scheduler: &NativeScheduler<IoUringSelector>,
            cancellation: &Cancellation,
        ) -> OperationResult<Option<Self::Message>> {
            self.0.recv(scheduler.selector(), cancellation).await
        }

        async fn close(
            &mut self,
            scheduler: &NativeScheduler<IoUringSelector>,
        ) -> std::io::Result<()> {
            self.0.close(scheduler.selector()).await
        }
    }

    impl Multishot for NativeScheduler<IoUringSelector> {
        type Receiver = Receiver;

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
                .map(Receiver)
        }
    }
}
