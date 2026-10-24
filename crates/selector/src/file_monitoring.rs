// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{NativeFile, ReadinessSelector};
use core_scheduler::backend::{
    BoundFileReader, BoundFileWriter, FileMonitor, FileMonitorSubmission, FileMonitoring,
};
use core_scheduler::{FileReading, FileWriting};

use std::io;

impl FileMonitoring for ReadinessSelector {
    type FileRegistration = ();

    fn file_monitor(&self, file: &NativeFile) -> io::Result<FileMonitor<NativeFile>> {
        Ok(FileMonitor::new(NativeFile(file.retain()), ()))
    }
}

impl FileMonitorSubmission for ReadinessSelector {}

impl FileReading for ReadinessSelector {
    type Reader<'scheduler> = BoundFileReader<'scheduler, Self>;
    fn file_reader(&self, file: &Self::File) -> io::Result<Self::Reader<'_>> {
        BoundFileReader::new(self, file)
    }
}

impl FileWriting for ReadinessSelector {
    type Writer<'scheduler> = BoundFileWriter<'scheduler, Self>;
    fn file_writer(&self, file: &Self::File) -> io::Result<Self::Writer<'_>> {
        BoundFileWriter::new(self, file)
    }
}
