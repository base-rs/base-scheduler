// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{IoBuf, IoBufMut};

/// A uniquely writable completed receive that can become independently retained read-only storage. Backends choose the lease and shared representation; freezing consumes mutable access, and clones of the frozen owner keep its storage unavailable for reuse. Neither transition requires copying payload bytes.
pub trait ReceiveBuffer: IoBufMut {
    type Frozen: IoBuf + Clone;
    fn freeze(self) -> Self::Frozen;
}
