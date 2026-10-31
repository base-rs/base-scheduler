// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Sequential descriptor transfers for adopted pipes. Pipes reject socket receive and send calls, so these use `read`, `readv`, `write` and `writev` on the descriptor retained by the backend's socket wrapper.

use socket2::Socket;
use std::{
    io::{self, IoSlice, IoSliceMut},
    os::fd::AsRawFd,
};

// Linux and the BSDs, including macOS, define `IOV_MAX` as 1024. Longer requests make a short transfer instead of failing with `EINVAL`, matching the standard library.
const MAX_VECTORS: usize = 1024;

// macOS rejects scalar transfers longer than `INT_MAX` with `EINVAL`; Linux already limits a single transfer below this. Longer requests make a short transfer, matching the standard library.
const MAX_LENGTH: usize = libc::c_int::MAX as usize - 1;

// Skip leading empty ranges so that the vector limit cannot hide nonempty storage and report a false end of file or zero write.
fn nonempty<T: std::ops::Deref<Target = [u8]>>(buffers: &[T]) -> usize {
    buffers
        .iter()
        .position(|buffer| !buffer.is_empty())
        .unwrap_or(buffers.len())
}

fn result_count(result: isize) -> io::Result<usize> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result as usize)
    }
}

pub(crate) fn read(pipe: &Socket, buffer: &mut [u8]) -> io::Result<usize> {
    // SAFETY: The exclusive byte range remains alive until this synchronous call returns; the kernel writes at most its length.
    result_count(unsafe {
        libc::read(
            pipe.as_raw_fd(),
            buffer.as_mut_ptr().cast(),
            buffer.len().min(MAX_LENGTH),
        )
    })
}

pub(crate) fn read_vectored(pipe: &Socket, buffers: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
    let start = nonempty(buffers);
    let buffers = &mut buffers[start..];
    // SAFETY: `IoSliceMut` is ABI-compatible with `iovec` on Unix. The exclusive ranges remain alive until this synchronous call returns.
    result_count(unsafe {
        libc::readv(
            pipe.as_raw_fd(),
            buffers.as_mut_ptr().cast(),
            buffers.len().min(MAX_VECTORS) as libc::c_int,
        )
    })
}

pub(crate) fn write(pipe: &Socket, buffer: &[u8]) -> io::Result<usize> {
    // SAFETY: The byte range remains alive until this synchronous call returns; the kernel reads at most its length.
    result_count(unsafe {
        libc::write(
            pipe.as_raw_fd(),
            buffer.as_ptr().cast(),
            buffer.len().min(MAX_LENGTH),
        )
    })
}

pub(crate) fn write_vectored(pipe: &Socket, buffers: &[IoSlice<'_>]) -> io::Result<usize> {
    let buffers = &buffers[nonempty(buffers)..];
    // SAFETY: `IoSlice` is ABI-compatible with `iovec` on Unix. The ranges remain alive until this synchronous call returns.
    result_count(unsafe {
        libc::writev(
            pipe.as_raw_fd(),
            buffers.as_ptr().cast(),
            buffers.len().min(MAX_VECTORS) as libc::c_int,
        )
    })
}

/// Complete an empty transfer without a syscall. macOS rejects `readv` with no vectors and reports `WouldBlock` for an empty write to a full pipe, but empty transfers never need data or capacity.
pub(crate) fn empty(
    cancellation: &base_scheduler::Cancellation,
) -> base_scheduler::OperationResult<usize> {
    cancellation.check()?;
    Ok(0)
}
