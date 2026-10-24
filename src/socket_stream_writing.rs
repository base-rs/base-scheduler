// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{
    BoundSocketStreamWriter, BufferPool, BufferPooling, IoBuf, SocketMonitoring, SocketOperation,
    SocketStreamOutput, SocketStreamWriter,
};
use std::io;

/// Construct a sequential stream output lane bound to this scheduler. Construction performs no writes and rejects non-stream sockets.
pub trait SocketStreamWriting: SocketOperation {
    #[doc(hidden)]
    type Writer<B: IoBuf>: SocketStreamOutput<Self, Buffer = B>;

    /// Construct a writer for one buffer owner type. Applications normally infer this type from `submit`; pooled leases can be repeatedly transferred without reallocating payload storage. Socket ownership and registration follow the backend's compatibility contract. Separate writers do not coordinate output through aliases of the same socket.
    #[doc(hidden)]
    fn socket_stream_output<B: IoBuf>(&self, socket: &Self::Socket) -> io::Result<Self::Writer<B>>;

    /// Retain the socket and reusable backend state. Owned submissions accept this scheduler's pooled chunks, so the default reader and writer compose without choosing a buffer representation. Borrowed writes do not allocate a payload pool. Subsequent operations take only payloads and cancellation, never another scheduler.
    fn socket_stream_writer<'scheduler>(
        &'scheduler self,
        socket: &Self::Socket,
    ) -> io::Result<impl SocketStreamWriter<Buffer = <Self::Pool as BufferPool>::Buffer> + 'scheduler>
    where
        Self: SocketMonitoring + BufferPooling + Sync,
        Self::Socket: 'static,
    {
        BoundSocketStreamWriter::new(self, socket)
    }

    /// Select another stable owner type, such as `Vec<u8>` or an application lease. This changes storage compatibility, not the writer's ordering or backend execution strategy.
    fn socket_stream_writer_with_buffer<'scheduler, B: IoBuf>(
        &'scheduler self,
        socket: &Self::Socket,
    ) -> io::Result<impl SocketStreamWriter<Buffer = B> + 'scheduler>
    where
        Self: SocketMonitoring + Sync,
        Self::Socket: 'static,
    {
        BoundSocketStreamWriter::new(self, socket)
    }
}
