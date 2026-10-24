// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::backend::{FileRead, FileReadAt, FileWrite, FileWriteAt};
use core_scheduler::{FileClose, FileOpen, FileSeek, FileStat, FileSync, FileTruncate};

/// The complete regular-file operation profile required by the shared full-suite fixture. Application code requests individual capabilities instead.
pub trait FileTestScheduler:
    FileOpen
    + FileClose
    + FileStat
    + FileTruncate
    + FileSync
    + FileSeek
    + FileRead
    + FileWrite
    + FileReadAt
    + FileWriteAt
{
}

impl<S: ?Sized> FileTestScheduler for S where
    S: FileOpen
        + FileClose
        + FileStat
        + FileTruncate
        + FileSync
        + FileSeek
        + FileRead
        + FileWrite
        + FileReadAt
        + FileWriteAt
{
}
