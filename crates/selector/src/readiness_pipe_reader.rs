// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{Interest, NativeSocket, ReadinessSelector, pipe_io};
use base_scheduler::{Cancellation, OperationResult, PipeReader, PipeReading, SocketClose};
use std::{
    io::{self, IoSliceMut},
    os::fd::OwnedFd,
};

/// Scheduler-bound reader owning an adopted pipe. Each attempt runs synchronously in the polling future; `WouldBlock` waits for descriptor readiness.
pub(crate) struct ReadinessPipeReader<'a> {
    scheduler: &'a ReadinessSelector,
    pipe: Option<NativeSocket>,
}

impl ReadinessPipeReader<'_> {
    fn pipe(&self) -> io::Result<&NativeSocket> {
        self.pipe
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "pipe reader is closed"))
    }
}

impl PipeReading for ReadinessSelector {
    fn pipe_reader<'scheduler>(
        &'scheduler self,
        pipe: OwnedFd,
    ) -> io::Result<impl PipeReader + 'scheduler + use<'scheduler>> {
        self.adopt_pipe(pipe).map(|pipe| ReadinessPipeReader {
            scheduler: self,
            pipe: Some(pipe),
        })
    }
}

impl PipeReader for ReadinessPipeReader<'_> {
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
            .socket_operation(
                pipe,
                buffer,
                cancellation,
                Interest::Readable,
                |pipe, buffer| pipe_io::read(pipe, buffer),
            )
            .await
            .0
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
            .socket_operation(
                pipe,
                buffers,
                cancellation,
                Interest::Readable,
                |pipe, buffers| pipe_io::read_vectored(pipe, buffers),
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
