// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{IoBuf, SocketRecvResult};

/// One received stream chunk or datagram, with metadata and an owned buffer exposing exactly its initialized payload.
///
/// The buffer can move to an owned write or outlive its receiver and scheduler. Its backend chooses how storage is allocated and recycled; polling and multishot receivers share this result type.
pub struct SocketRecvMessage<Buffer: IoBuf> {
    pub result: SocketRecvResult,
    pub buffer: Buffer,
}
