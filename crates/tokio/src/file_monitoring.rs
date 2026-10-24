// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{TokioFile, TokioScheduler};
use core_scheduler::backend::{
    BoundFileReader, BoundFileWriter, FileMonitor, FileMonitorSubmission, FileMonitoring,
};
use core_scheduler::{FileReading, FileWriting};

use std::io;

impl FileMonitoring for TokioScheduler {
    type FileRegistration = ();

    fn file_monitor(&self, file: &TokioFile) -> io::Result<FileMonitor<TokioFile>> {
        Ok(FileMonitor::new(TokioFile(file.retain()), ()))
    }
}

impl FileMonitorSubmission for TokioScheduler {}

impl FileReading for TokioScheduler {
    type Reader<'scheduler> = BoundFileReader<'scheduler, Self>;
    fn file_reader(&self, file: &Self::File) -> io::Result<Self::Reader<'_>> {
        BoundFileReader::new(self, file)
    }
}

impl FileWriting for TokioScheduler {
    type Writer<'scheduler> = BoundFileWriter<'scheduler, Self>;
    fn file_writer(&self, file: &Self::File) -> io::Result<Self::Writer<'_>> {
        BoundFileWriter::new(self, file)
    }
}
