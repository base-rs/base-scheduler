// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{TokioFile, TokioScheduler};
use core_scheduler::{FileMonitor, FileMonitoring};
use std::io;

impl FileMonitoring for TokioScheduler {
    type FileRegistration = ();

    fn file_monitor(&self, file: &TokioFile) -> io::Result<FileMonitor<TokioFile>> {
        Ok(FileMonitor::new(TokioFile(file.retain()), ()))
    }
}
