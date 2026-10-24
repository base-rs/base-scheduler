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
