// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{DirectScheduler, DirectSocket, pipe_io};
use base_scheduler::{Cancellation, OperationResult, PipeReader, PipeReading, SocketClose};
use std::{
    io::{self, IoSliceMut},
    os::fd::OwnedFd,
};

/// Scheduler-bound reader owning an adopted pipe. Each attempt runs synchronously in the polling future; `WouldBlock` waits for the scheduler's retry pulse.
pub(crate) struct DirectPipeReader<'a> {
    scheduler: &'a DirectScheduler,
    pipe: Option<DirectSocket>,
}

impl DirectPipeReader<'_> {
    fn pipe(&self) -> io::Result<&DirectSocket> {
        self.pipe
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "pipe reader is closed"))
    }
}

impl PipeReading for DirectScheduler {
    fn pipe_reader<'scheduler>(
        &'scheduler self,
        pipe: OwnedFd,
    ) -> io::Result<impl PipeReader + 'scheduler + use<'scheduler>> {
        self.adopt_pipe(pipe).map(|pipe| DirectPipeReader {
            scheduler: self,
            pipe: Some(pipe),
        })
    }
}

impl PipeReader for DirectPipeReader<'_> {
    async fn read_into(
        &mut self,
        buffer: &mut [u8],
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        let pipe = self.pipe()?;
        if buffer.is_empty() {
            return pipe_io::empty(cancellation);
        }
        self.scheduler
            .socket_poll_operation(pipe, cancellation, |pipe| pipe_io::read(pipe, buffer))
            .await
    }

    async fn read_vectored(
        &mut self,
        buffers: &mut [IoSliceMut<'_>],
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        let pipe = self.pipe()?;
        if buffers.iter().all(|buffer| buffer.is_empty()) {
            return pipe_io::empty(cancellation);
        }
        self.scheduler
            .socket_poll_operation(pipe, cancellation, |pipe| {
                pipe_io::read_vectored(pipe, buffers)
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
