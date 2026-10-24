// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{BoundSocketStreamReader, BufferPooling, SocketMonitoring};
use std::io;

/// Construct a sequential stream input lane bound to this scheduler. Protocol consumers can depend on [`SocketStreamReader`](crate::SocketStreamReader) without requiring unrelated scheduler capabilities.
pub trait SocketStreamReading: SocketMonitoring + BufferPooling + Sync {
    /// Retain the socket and allocate persistent registration state once. No receive or payload-pool allocation occurs during construction. Use `with_pool` on the returned reader to share a receive budget across connections.
    fn socket_stream_reader(
        &self,
        socket: &Self::Socket,
    ) -> io::Result<BoundSocketStreamReader<'_, Self>>
    where
        Self::Socket: 'static,
    {
        BoundSocketStreamReader::new(self, socket)
    }
}
impl<S: SocketMonitoring + BufferPooling + Sync + ?Sized> SocketStreamReading for S {}
