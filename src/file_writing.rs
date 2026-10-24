// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{BoundFileWriter, FileMonitoring};
use std::io;

/// Construct a reusable file writer bound to this scheduler. Construction retains the file and backend monitor state without submitting I/O. Cancellation remains an operation argument.
pub trait FileWriting: FileMonitoring + Sync {
    fn file_writer(&self, file: &Self::File) -> io::Result<BoundFileWriter<'_, Self>> {
        Ok(BoundFileWriter {
            scheduler: self,
            monitor: self.file_monitor(file)?,
        })
    }
}

impl<S: FileMonitoring + Sync + ?Sized> FileWriting for S {}
