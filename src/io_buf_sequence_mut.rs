// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{IoBufMut, IoBufSequence};

/// An owned sequence of writable buffers. Receiving fills their initialized byte ranges without resizing them or replacing their owners.
///
/// # Safety
/// `buffers_mut()` must expose the same buffer owners, order, and length as `buffers()`, with exclusive access. Each buffer must uphold `IoBufMut`; no other owner may access the writable byte ranges during an operation. A backend must not replace or invalidate buffer owners while their byte storage may still be accessed by a submitted operation.
pub unsafe trait IoBufSequenceMut: IoBufSequence<Buffer: IoBufMut> {
    fn buffers_mut(&mut self) -> &mut [Self::Buffer];
}

// SAFETY: Vec gives exclusive access to its elements, each of which implements IoBufMut.
unsafe impl<B: IoBufMut> IoBufSequenceMut for Vec<B> {
    fn buffers_mut(&mut self) -> &mut [B] {
        self.as_mut_slice()
    }
}

// SAFETY: Arrays give exclusive access to their elements, each of which implements IoBufMut.
unsafe impl<B: IoBufMut, const N: usize> IoBufSequenceMut for [B; N] {
    fn buffers_mut(&mut self) -> &mut [B] {
        self
    }
}

// SAFETY: The boxed slice gives exclusive access to its elements, each of which implements IoBufMut.
unsafe impl<B: IoBufMut> IoBufSequenceMut for Box<[B]> {
    fn buffers_mut(&mut self) -> &mut [B] {
        self
    }
}
