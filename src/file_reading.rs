// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{FileMonitoring, MonitoredFileReader};
use std::io;

/// Construct a reusable file reader bound to this scheduler. Construction retains the file and backend monitor state without submitting I/O. Cancellation remains an operation argument.
pub trait FileReading: FileMonitoring + Sync {
    fn file_reader(&self, file: &Self::File) -> io::Result<MonitoredFileReader<'_, Self>> {
        Ok(MonitoredFileReader {
            scheduler: self,
            monitor: self.file_monitor(file)?,
        })
    }
}

impl<S: FileMonitoring + Sync + ?Sized> FileReading for S {}
