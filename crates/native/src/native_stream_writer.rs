// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{NativeScheduler, Selector};
use core_scheduler::{
    Cancellation, IoBuf, OperationError, OperationResult, SocketStreamWriter, SocketStreamWriting,
};
use std::io;

/// Ordered selector output driven through an explicitly supplied native scheduler.
pub struct NativeStreamWriter<W>(W);

impl<S, W> SocketStreamWriter<NativeScheduler<S>> for NativeStreamWriter<W>
where
    S: Selector,
    W: SocketStreamWriter<S>,
{
    type Buffer = W::Buffer;

    async fn submit(
        &mut self,
        scheduler: &NativeScheduler<S>,
        buffer: Self::Buffer,
        cancellation: &Cancellation,
    ) -> Result<(), (OperationError, Self::Buffer)> {
        self.0
            .submit(scheduler.selector(), buffer, cancellation)
            .await
    }

    async fn flush(
        &mut self,
        scheduler: &NativeScheduler<S>,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        self.0.flush(scheduler.selector(), cancellation).await
    }

    async fn close(&mut self, scheduler: &NativeScheduler<S>) -> io::Result<()> {
        self.0.close(scheduler.selector()).await
    }
}

impl<S: Selector + SocketStreamWriting> SocketStreamWriting for NativeScheduler<S> {
    type Writer<B: IoBuf> = NativeStreamWriter<S::Writer<B>>;

    fn socket_stream_writer<B: IoBuf>(&self, socket: &Self::Socket) -> io::Result<Self::Writer<B>> {
        self.selector()
            .socket_stream_writer(socket)
            .map(NativeStreamWriter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_scheduler::SocketOperation;
    use std::{
        marker::PhantomData,
        sync::{Arc, Mutex},
    };

    struct RecordingSelector(Arc<Mutex<Vec<u8>>>);
    impl Selector for RecordingSelector {
        fn close(&self) {}
        async fn shutdown(&self) {}
    }
    impl SocketOperation for RecordingSelector {
        type Socket = u8;
    }
    struct RecordingWriter<B>(u8, PhantomData<B>);
    impl SocketStreamWriting for RecordingSelector {
        type Writer<B: IoBuf> = RecordingWriter<B>;
        fn socket_stream_writer<B: IoBuf>(&self, socket: &u8) -> io::Result<Self::Writer<B>> {
            Ok(RecordingWriter(*socket, PhantomData))
        }
    }
    impl<B: IoBuf> SocketStreamWriter<RecordingSelector> for RecordingWriter<B> {
        type Buffer = B;
        async fn submit(
            &mut self,
            scheduler: &RecordingSelector,
            buffer: B,
            cancellation: &Cancellation,
        ) -> Result<(), (OperationError, B)> {
            if let Err(error) = cancellation.check() {
                return Err((error.into(), buffer));
            }
            let mut output = scheduler.0.lock().unwrap();
            output.push(self.0);
            output.extend_from_slice(buffer.as_ref());
            Ok(())
        }
        async fn flush(
            &mut self,
            scheduler: &RecordingSelector,
            cancellation: &Cancellation,
        ) -> OperationResult<()> {
            cancellation.check()?;
            scheduler.0.lock().unwrap().push(b'f');
            Ok(())
        }
        async fn close(&mut self, scheduler: &RecordingSelector) -> io::Result<()> {
            scheduler.0.lock().unwrap().push(b'c');
            Ok(())
        }
    }

    #[test]
    fn forwarding_preserves_explicit_services_tokens_and_owners() {
        let first = NativeScheduler::with_selector(
            RecordingSelector(Arc::default()),
            crate::NativeExecutorOptions::default(),
        )
        .unwrap();
        let second = NativeScheduler::with_selector(
            RecordingSelector(Arc::default()),
            crate::NativeExecutorOptions::default(),
        )
        .unwrap();
        first.block_on(async {
            let never = Cancellation::never();
            let cancelled = Cancellation::new();
            cancelled.cancel();
            let mut writer = first.socket_stream_writer(&b's').unwrap();
            writer
                .submit(&second, b"data".to_vec(), &never)
                .await
                .unwrap();
            let (error, buffer) = writer
                .submit(&second, b"returned".to_vec(), &cancelled)
                .await
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::Interrupted);
            assert_eq!(buffer, b"returned");
            assert!(writer.flush(&second, &cancelled).await.is_err());
            writer.flush(&second, &never).await.unwrap();
            writer.close(&second).await.unwrap();
            assert!(first.selector().0.lock().unwrap().is_empty());
            assert_eq!(*second.selector().0.lock().unwrap(), b"sdatafc");
            second.shutdown().await;
            first.shutdown().await;
        });
    }
}
