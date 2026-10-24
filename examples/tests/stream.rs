// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::*;
use core_scheduler_direct::DirectScheduler;
use core_scheduler_examples::stream;
use futures_lite::future::{block_on, poll_once};
use std::{
    io::{self, Write},
    net::TcpListener,
    pin::pin,
};

struct FailedMonitor(bool);
impl SocketOperation for FailedMonitor {
    type Socket = ();
}
impl BufferPooling for FailedMonitor {
    type Pool = HeapBufferPool;
    fn buffer_pool(&self, options: BufferPoolOptions) -> io::Result<Self::Pool> {
        HeapBufferPool::with_options(options)
    }
}
impl SocketMonitoring for FailedMonitor {
    fn socket_type(&self, _: &()) -> io::Result<SocketType> {
        Ok(SocketType::Stream)
    }

    type MonitorRegistration = ();
    fn socket_monitor_state(
        &self,
        _: &(),
        interest: Interest,
    ) -> io::Result<SocketMonitor<(), ()>> {
        if self.0 {
            Err(io::ErrorKind::PermissionDenied.into())
        } else {
            Ok(SocketMonitor::new((), interest, ()))
        }
    }
    async fn socket_monitor_close<M: SocketMonitorState<(), ()>>(
        &self,
        _: &mut MonitorStorage<M>,
    ) -> io::Result<()> {
        Err(io::ErrorKind::BrokenPipe.into())
    }
}
impl SocketRecvPoll for FailedMonitor {
    async fn socket_recv_poll(
        &self,
        _: &(),
        _: &mut [u8],
        _: SocketRecvFlags,
        _: &Cancellation,
    ) -> OperationResult<usize> {
        Ok(0)
    }
}
impl SocketSendPoll for FailedMonitor {
    async fn socket_send_poll(
        &self,
        _: &(),
        _: &[u8],
        _: SocketSendFlags,
        _: &Cancellation,
    ) -> OperationResult<usize> {
        unreachable!()
    }
}
impl SocketRecvMsgPoll for FailedMonitor {
    async fn socket_recvmsg_poll(
        &self,
        _: &(),
        _: &mut [std::io::IoSliceMut<'_>],
        _: SocketRecvFlags,
        _: &Cancellation,
    ) -> OperationResult<SocketRecvResult> {
        unreachable!()
    }
}
impl SocketSendMsgPoll for FailedMonitor {
    async fn socket_sendmsg_poll(
        &self,
        _: &(),
        _: &[std::io::IoSlice<'_>],
        _: Option<SocketAddress>,
        _: SocketSendFlags,
        _: &Cancellation,
    ) -> OperationResult<usize> {
        unreachable!()
    }
}
impl SocketWait for FailedMonitor {
    async fn socket_wait(&self, _: &(), _: Interest, _: &Cancellation) -> OperationResult<()> {
        unreachable!()
    }
}

#[test]
fn stream_reports_monitor_setup_and_close_failures() {
    block_on(async {
        for fail_setup in [false, true] {
            let error = stream::receive(
                &FailedMonitor(fail_setup),
                &(),
                &mut [0; 1],
                &Cancellation::never(),
                |_| Ok(()),
            )
            .await
            .unwrap_err();
            assert_eq!(
                error.kind(),
                if fail_setup {
                    io::ErrorKind::PermissionDenied
                } else {
                    io::ErrorKind::BrokenPipe
                }
            );
        }
    });
}

#[test]
fn stream_cleanup_allows_exclusive_close_after_cancellation_and_consumer_errors() {
    block_on(async {
        let scheduler = DirectScheduler::new(Default::default()).unwrap();
        for mode in 0..3 {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let socket = scheduler
                .socket_open(
                    SocketDomain::Ipv4,
                    SocketType::Stream,
                    None,
                    &Cancellation::never(),
                )
                .await
                .unwrap();
            scheduler
                .socket_connect(
                    &socket,
                    listener.local_addr().unwrap(),
                    &Cancellation::never(),
                )
                .await
                .unwrap();
            let (mut peer, _) = listener.accept().unwrap();
            let token = Cancellation::new();
            let mut storage = [0; 16];
            let result = if mode == 0 {
                let mut receive = pin!(stream::receive(
                    &scheduler,
                    &socket,
                    &mut storage,
                    &token,
                    |_| Ok(())
                ));
                assert!(poll_once(receive.as_mut()).await.is_none());
                token.cancel();
                receive.await
            } else if mode == 1 {
                peer.write_all(b"data").unwrap();
                stream::receive(&scheduler, &socket, &mut storage, &token, |_| {
                    Err(io::Error::from(io::ErrorKind::InvalidData).into())
                })
                .await
            } else {
                stream::receive(&scheduler, &socket, &mut [], &token, |_| Ok(())).await
            };
            assert_eq!(
                result.unwrap_err().kind(),
                match mode {
                    0 => io::ErrorKind::Interrupted,
                    1 => io::ErrorKind::InvalidData,
                    _ => io::ErrorKind::InvalidInput,
                }
            );
            scheduler.socket_close(socket).await.unwrap();
        }
        scheduler.shutdown().await;
    });
}
