// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{IoBuf, SocketOperation, SocketStreamOutput};
use std::io;

/// Construct backend-owned ordered output with explicit scheduler dispatch. Applications use [`SocketStreamWriting`](crate::SocketStreamWriting); this hook supports default bound-writer composition without imposing that implementation on specialized backends.
#[doc(hidden)]
pub trait SocketStreamOutputting: SocketOperation {
    type Writer<B: IoBuf>: SocketStreamOutput<Self, Buffer = B>;

    fn socket_stream_output<B: IoBuf>(&self, socket: &Self::Socket) -> io::Result<Self::Writer<B>>;
}
