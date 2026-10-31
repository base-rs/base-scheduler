// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{DirectScheduler, DirectSocket, pipe_io};
use base_scheduler::{Cancellation, OperationResult, PipeWriter, PipeWriting, SocketClose};
use std::{
    io::{self, IoSlice},
    os::fd::OwnedFd,
};

/// Scheduler-bound writer owning an adopted pipe. Each attempt runs synchronously in the polling future; `WouldBlock` waits for the scheduler's retry pulse.
pub(crate) struct DirectPipeWriter<'a> {
    scheduler: &'a DirectScheduler,
    pipe: Option<DirectSocket>,
}

impl DirectPipeWriter<'_> {
    fn pipe(&self) -> io::Result<&DirectSocket> {
        self.pipe
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "pipe writer is closed"))
    }
}

impl PipeWriting for DirectScheduler {
    fn pipe_writer<'scheduler>(
        &'scheduler self,
        pipe: OwnedFd,
    ) -> io::Result<impl PipeWriter + 'scheduler + use<'scheduler>> {
        self.adopt_pipe(pipe).map(|pipe| DirectPipeWriter {
            scheduler: self,
            pipe: Some(pipe),
        })
    }
}

impl PipeWriter for DirectPipeWriter<'_> {
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
            .socket_poll_operation(pipe, cancellation, |pipe| pipe_io::write(pipe, buffer))
            .await
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
            .socket_poll_operation(pipe, cancellation, |pipe| {
                pipe_io::write_vectored(pipe, buffers)
            })
            .await
    }

    async fn close(&mut self) -> io::Result<()> {
        match self.pipe.take() {
            Some(pipe) => self.scheduler.socket_close(pipe).await,
            None => Ok(()),
        }
    }
}
