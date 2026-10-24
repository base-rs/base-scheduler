// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{IoBuf, SocketOperation, SocketStreamWriter};
use std::io;

/// Construct reusable ordered output state for stream sockets. Construction performs no writes and rejects non-stream sockets. The associated writer retains resource and backend state, while operations continue to receive execution services explicitly.
pub trait SocketStreamWriting: SocketOperation {
    type Writer<B: IoBuf>: SocketStreamWriter<Self, Buffer = B>;

    /// Construct a writer for one buffer owner type. Applications normally infer this type from `submit`; pooled leases can be repeatedly transferred without reallocating payload storage. Socket ownership and registration follow the backend's compatibility contract. Separate writers do not coordinate output through aliases of the same socket.
    fn socket_stream_writer<B: IoBuf>(
        &self,
        socket: &Self::Socket,
    ) -> io::Result<Self::Writer<B>>;
}
