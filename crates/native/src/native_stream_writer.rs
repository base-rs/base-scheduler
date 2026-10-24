// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{NativeScheduler, Selector};
use core_scheduler::{
    BufferPool, Cancellation, IoBuf, OperationError, OperationResult, SocketStreamOutput,
    SocketStreamOutputting, SocketStreamWriter, SocketStreamWriting,
};
use std::io;

/// Ordered selector output driven through an explicitly supplied native scheduler.
pub struct NativeStreamWriter<W>(W);

impl<S, W> SocketStreamOutput<NativeScheduler<S>> for NativeStreamWriter<W>
where
    S: Selector,
    W: SocketStreamOutput<S>,
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

impl<S: Selector + SocketStreamOutputting> SocketStreamOutputting for NativeScheduler<S> {
    type Writer<B: IoBuf> = NativeStreamWriter<S::Writer<B>>;

    fn socket_stream_output<B: IoBuf>(&self, socket: &Self::Socket) -> io::Result<Self::Writer<B>> {
        self.selector()
            .socket_stream_output(socket)
            .map(NativeStreamWriter)
    }
}

impl<S: Selector + SocketStreamWriting> SocketStreamWriting for NativeScheduler<S> {
    fn socket_stream_writer<'scheduler>(
        &'scheduler self,
        socket: &Self::Socket,
    ) -> io::Result<impl SocketStreamWriter<Buffer = <Self::Pool as BufferPool>::Buffer> + 'scheduler>
    where
        Self::Socket: 'static,
    {
        self.selector().socket_stream_writer(socket)
    }

    fn socket_stream_writer_with_buffer<'scheduler, B: IoBuf>(
        &'scheduler self,
        socket: &Self::Socket,
    ) -> io::Result<impl SocketStreamWriter<Buffer = B> + 'scheduler>
    where
        Self::Socket: 'static,
    {
        self.selector().socket_stream_writer_with_buffer(socket)
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
    impl SocketStreamOutputting for RecordingSelector {
        type Writer<B: IoBuf> = RecordingWriter<B>;
        fn socket_stream_output<B: IoBuf>(&self, socket: &u8) -> io::Result<Self::Writer<B>> {
            Ok(RecordingWriter(*socket, PhantomData))
        }
    }
    impl<B: IoBuf> SocketStreamOutput<RecordingSelector> for RecordingWriter<B> {
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

    struct BoundSelector(RecordingSelector);
    impl Selector for BoundSelector {
        fn close(&self) {}
        async fn shutdown(&self) {}
    }
    impl SocketOperation for BoundSelector {
        type Socket = u8;
    }
    impl core_scheduler::BufferPooling for BoundSelector {
        type Pool = core_scheduler::HeapBufferPool;
        fn buffer_pool(
            &self,
            options: core_scheduler::BufferPoolOptions,
        ) -> io::Result<Self::Pool> {
            core_scheduler::HeapBufferPool::with_options(options)
        }
    }

    struct BoundRecordingWriter<'a, B> {
        selector: &'a RecordingSelector,
        output: RecordingWriter<B>,
    }
    impl<B: IoBuf> SocketStreamWriter for BoundRecordingWriter<'_, B> {
        type Buffer = B;
        async fn write(
            &mut self,
            buffer: &[u8],
            cancellation: &Cancellation,
        ) -> OperationResult<usize> {
            cancellation.check()?;
            self.selector.0.lock().unwrap().extend_from_slice(buffer);
            Ok(buffer.len())
        }
        async fn write_vectored(
            &mut self,
            buffers: &[std::io::IoSlice<'_>],
            cancellation: &Cancellation,
        ) -> OperationResult<usize> {
            cancellation.check()?;
            let mut output = self.selector.0.lock().unwrap();
            let mut bytes = 0;
            for buffer in buffers {
                output.extend_from_slice(buffer);
                bytes += buffer.len();
            }
            Ok(bytes)
        }
        async fn submit(
            &mut self,
            buffer: B,
            cancellation: &Cancellation,
        ) -> Result<(), (OperationError, B)> {
            self.output
                .submit(self.selector, buffer, cancellation)
                .await
        }
        async fn flush(&mut self, cancellation: &Cancellation) -> OperationResult<()> {
            self.output.flush(self.selector, cancellation).await
        }
        async fn close(&mut self) -> io::Result<()> {
            self.output.close(self.selector).await
        }
    }

    // This selector has neither raw output nor monitoring capabilities. Only its public writer factories are forwarded.
    impl SocketStreamWriting for BoundSelector {
        fn socket_stream_writer<'scheduler>(
            &'scheduler self,
            socket: &u8,
        ) -> io::Result<
            impl SocketStreamWriter<Buffer = <Self::Pool as BufferPool>::Buffer> + 'scheduler,
        > {
            self.0.0.lock().unwrap().push(b'D');
            Ok(BoundRecordingWriter {
                selector: &self.0,
                output: RecordingWriter(*socket, PhantomData),
            })
        }
        fn socket_stream_writer_with_buffer<'scheduler, B: IoBuf>(
            &'scheduler self,
            socket: &u8,
        ) -> io::Result<impl SocketStreamWriter<Buffer = B> + 'scheduler> {
            self.0.0.lock().unwrap().push(b'T');
            Ok(BoundRecordingWriter {
                selector: &self.0,
                output: RecordingWriter(*socket, PhantomData),
            })
        }
    }

    #[test]
    fn public_factories_preserve_selector_specialization_without_monitor_capabilities() {
        use core_scheduler::{BufferPoolOptions, BufferPooling};
        let scheduler = NativeScheduler::with_selector(
            BoundSelector(RecordingSelector(Arc::default())),
            crate::NativeExecutorOptions::default(),
        )
        .unwrap();
        scheduler.block_on(async {
            let never = Cancellation::never();
            let pool = scheduler.buffer_pool(BufferPoolOptions::new(1, 8)).unwrap();
            let mut buffer = pool.acquire(&never).await.unwrap();
            buffer.as_mut()[..5].copy_from_slice(b"owned");
            buffer.truncate(5);
            let mut writer = {
                let socket = b'd';
                scheduler.socket_stream_writer(&socket).unwrap()
            };
            let cancelled = Cancellation::new();
            cancelled.cancel();
            assert!(writer.write(b"rejected", &cancelled).await.is_err());
            assert!(
                writer
                    .write_vectored(&[std::io::IoSlice::new(b"rejected")], &cancelled)
                    .await
                    .is_err()
            );
            writer.submit(buffer, &never).await.unwrap();
            writer.write(b"borrowed", &never).await.unwrap();
            writer
                .write_vectored(
                    &[
                        std::io::IoSlice::new(b"vec"),
                        std::io::IoSlice::new(b"tors"),
                    ],
                    &never,
                )
                .await
                .unwrap();
            writer.flush(&never).await.unwrap();
            writer.close().await.unwrap();
            drop(writer);
            let mut writer = {
                let socket = b't';
                scheduler
                    .socket_stream_writer_with_buffer::<Vec<u8>>(&socket)
                    .unwrap()
            };
            writer.submit(b"typed".to_vec(), &never).await.unwrap();
            writer.flush(&never).await.unwrap();
            writer.close().await.unwrap();
            assert_eq!(
                *scheduler.selector().0.0.lock().unwrap(),
                b"DdownedborrowedvectorsfcTttypedfc"
            );
            scheduler.shutdown().await;
        });
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
            let mut writer = first.socket_stream_output(&b's').unwrap();
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
