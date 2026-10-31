// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{Interest, NativeSocket, ReadinessSelector, pipe_io};
use base_scheduler::{Cancellation, OperationResult, PipeWriter, PipeWriting, SocketClose};
use std::{
    io::{self, IoSlice},
    os::fd::OwnedFd,
};

/// Scheduler-bound writer owning an adopted pipe. Each attempt runs synchronously in the polling future; `WouldBlock` waits for descriptor readiness.
pub(crate) struct ReadinessPipeWriter<'a> {
    scheduler: &'a ReadinessSelector,
    pipe: Option<NativeSocket>,
}

impl ReadinessPipeWriter<'_> {
    fn pipe(&self) -> io::Result<&NativeSocket> {
        self.pipe
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "pipe writer is closed"))
    }
}

impl PipeWriting for ReadinessSelector {
    fn pipe_writer<'scheduler>(
        &'scheduler self,
        pipe: OwnedFd,
    ) -> io::Result<impl PipeWriter + 'scheduler + use<'scheduler>> {
        self.adopt_pipe(pipe).map(|pipe| ReadinessPipeWriter {
            scheduler: self,
            pipe: Some(pipe),
        })
    }
}

impl PipeWriter for ReadinessPipeWriter<'_> {
    async fn write(
        &mut self,
        buffer: &[u8],
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        let pipe = self.pipe()?;
        if buffer.is_empty() {
            return pipe_io::empty(cancellation);
        }
        self.scheduler
            .socket_operation(
                pipe,
                buffer,
                cancellation,
                Interest::Writable,
                |pipe, buffer| pipe_io::write(pipe, buffer),
            )
            .await
            .0
    }

    async fn write_vectored(
        &mut self,
        buffers: &[IoSlice<'_>],
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        let pipe = self.pipe()?;
        if buffers.iter().all(|buffer| buffer.is_empty()) {
            return pipe_io::empty(cancellation);
        }
        self.scheduler
            .socket_operation(
                pipe,
                buffers,
                cancellation,
                Interest::Writable,
                |pipe, buffers| pipe_io::write_vectored(pipe, buffers),
            )
            .await
            .0
    }

    async fn close(&mut self) -> io::Result<()> {
        match self.pipe.take() {
            Some(pipe) => self.scheduler.socket_close(pipe).await,
            None => Ok(()),
        }
    }
}
