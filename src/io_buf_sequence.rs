// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::IoBuf;

/// An owned sequence of buffers for scatter/gather I/O. The collection itself may move; each buffer's byte storage must remain stable. Arrays avoid allocating a separate collection, and custom owners can retain pooled storage.
///
/// Inline byte arrays are not stable buffer owners, even when the outer collection is an array:
///
/// ```compile_fail
/// use protocol_scheduler::IoBufSequence;
/// fn accepts_sequence(_: impl IoBufSequence) {}
/// accepts_sequence([[0u8; 16]; 2]);
/// ```
///
/// # Safety
/// While an operation owns the sequence, `buffers()` must expose the same buffer owners in the same order and with the same length. Their `IoBuf` guarantees must remain valid until the sequence is dropped, including after the waiting future is abandoned. Interior mutation or another owner must not replace, remove, or invalidate those buffers. A backend may explicitly modify the sequence through exclusive access only when it no longer retains pointers into the affected buffers, such as before submission or after retirement.
pub unsafe trait IoBufSequence: Send + 'static {
    type Buffer: IoBuf;

    fn buffers(&self) -> &[Self::Buffer];
}

// SAFETY: These collections retain their elements and order as they move, and each element supplies stable byte storage through IoBuf.
unsafe impl<B: IoBuf> IoBufSequence for Vec<B> {
    type Buffer = B;

    fn buffers(&self) -> &[B] {
        self.as_slice()
    }
}

// SAFETY: Moving the array moves its buffer owners, without moving their byte storage or changing the sequence.
unsafe impl<B: IoBuf, const N: usize> IoBufSequence for [B; N] {
    type Buffer = B;

    fn buffers(&self) -> &[B] {
        self
    }
}

// SAFETY: The boxed slice retains its buffer owners and order until drop.
unsafe impl<B: IoBuf> IoBufSequence for Box<[B]> {
    type Buffer = B;

    fn buffers(&self) -> &[B] {
        self
    }
}
