// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{NativeFile, ReadinessSelector};
use core_scheduler::{FileMonitor, FileMonitoring};
use std::io;

impl FileMonitoring for ReadinessSelector {
    type FileRegistration = ();

    fn file_monitor(&self, file: &NativeFile) -> io::Result<FileMonitor<NativeFile>> {
        Ok(FileMonitor::new(NativeFile(file.retain()), ()))
    }
}

impl core_scheduler::FileMonitorSubmission for ReadinessSelector {}
