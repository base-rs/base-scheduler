// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{IoBuf, SocketRecvResult};

/// One message with an owned lease exposing exactly its initialized payload.
///
/// The lease can move to an owned write or outlive its receiver and scheduler. Dropping it returns storage to the receive pool without a separate recycle call.
pub struct RecvMsgMultishotMessage<Buffer: IoBuf> {
    pub result: SocketRecvResult,
    pub buffer: Buffer,
}
