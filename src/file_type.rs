// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// The portable classification of a filesystem object. Platform metadata can distinguish additional kinds, such as Unix devices, sockets, and FIFOs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileType {
    /// A regular file.
    File,
    /// A directory.
    Directory,
    /// A symbolic link.
    Symlink,
    /// Any other kind of object, such as a Unix device, socket, or FIFO.
    Other,
}
