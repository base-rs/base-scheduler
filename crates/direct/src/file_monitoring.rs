// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{DirectFile, DirectScheduler};
use core_scheduler::backend::{
    BoundFileReader, BoundFileWriter, FileMonitor, FileMonitorSubmission, FileMonitoring,
};
use core_scheduler::{FileReading, FileWriting};

use std::io;

impl FileMonitoring for DirectScheduler {
    type FileRegistration = ();

    fn file_monitor(&self, file: &DirectFile) -> io::Result<FileMonitor<DirectFile>> {
        Ok(FileMonitor::new(DirectFile(file.retain()), ()))
    }
}

impl FileMonitorSubmission for DirectScheduler {}

impl FileReading for DirectScheduler {
    type Reader<'scheduler> = BoundFileReader<'scheduler, Self>;
    fn file_reader(&self, file: &Self::File) -> io::Result<Self::Reader<'_>> {
        BoundFileReader::new(self, file)
    }
}

impl FileWriting for DirectScheduler {
    type Writer<'scheduler> = BoundFileWriter<'scheduler, Self>;
    fn file_writer(&self, file: &Self::File) -> io::Result<Self::Writer<'_>> {
        BoundFileWriter::new(self, file)
    }
}
