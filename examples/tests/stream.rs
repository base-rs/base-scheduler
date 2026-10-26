// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use base_scheduler::*;
use base_scheduler_direct::DirectScheduler;
use base_scheduler_examples::stream;
use futures_lite::future::{block_on, poll_once};
use std::{
    io::{self, Write},
    net::TcpListener,
    pin::pin,
};

struct FailedReaderFactory(bool);
impl SocketOperation for FailedReaderFactory {
    type Socket = ();
}
impl BufferPooling for FailedReaderFactory {
    type Pool = HeapBufferPool;
    fn buffer_pool(&self, options: BufferPoolOptions) -> io::Result<Self::Pool> {
        HeapBufferPool::with_options(options)
    }
}
struct EmptyReader;
impl SocketStreamReader for EmptyReader {
    type Buffer = BufferLease;
    async fn read(&mut self, _: &Cancellation) -> OperationResult<Option<Self::Buffer>> {
        Ok(None)
    }
    async fn read_into(&mut self, _: &mut [u8], _: &Cancellation) -> OperationResult<usize> {
        Ok(0)
    }
    async fn read_vectored(
        &mut self,
        _: &mut [io::IoSliceMut<'_>],
        _: &Cancellation,
    ) -> OperationResult<usize> {
        Ok(0)
    }
    async fn read_vectored_with_flags(
        &mut self,
        _: &mut [io::IoSliceMut<'_>],
        _: SocketRecvFlags,
        _: &Cancellation,
    ) -> OperationResult<SocketRecvResult> {
        unreachable!("This recipe only reads contiguous storage")
    }
    async fn read_owned_vectored<B: IoBufSequenceMut>(
        &mut self,
        _: B,
        _: SocketRecvFlags,
        _: &Cancellation,
    ) -> (OperationResult<SocketRecvResult>, B) {
        unreachable!("This recipe only reads contiguous storage")
    }
    async fn close(&mut self) -> io::Result<()> {
        Err(io::ErrorKind::BrokenPipe.into())
    }
}

#[test]
fn stream_reports_reader_setup_and_close_failures() {
    block_on(async {
        for fail_setup in [false, true] {
            let error = stream::receive(
                &FailedReaderFactory(fail_setup),
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

impl SocketStreamReading for FailedReaderFactory {
    fn socket_stream_reader(
        &self,
        _: &(),
    ) -> io::Result<impl SocketStreamReader<Buffer = BufferLease> + '_> {
        if self.0 {
            Err(io::ErrorKind::PermissionDenied.into())
        } else {
            Ok(EmptyReader)
        }
    }
    fn socket_stream_reader_with_pool(
        &self,
        socket: &(),
        _: HeapBufferPool,
    ) -> io::Result<impl SocketStreamReader<Buffer = BufferLease> + '_> {
        self.socket_stream_reader(socket)
    }
}
