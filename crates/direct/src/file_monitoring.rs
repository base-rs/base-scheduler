// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{DirectFile, DirectScheduler};
use core_scheduler::{FileMonitor, FileMonitoring};
use std::io;

impl FileMonitoring for DirectScheduler {
    type FileRegistration = ();

    fn file_monitor(&self, file: &DirectFile) -> io::Result<FileMonitor<DirectFile>> {
        Ok(FileMonitor::new(DirectFile(file.retain()), ()))
    }
}

impl core_scheduler::FileMonitorSubmission for DirectScheduler {}
