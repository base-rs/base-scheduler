// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{Cancellation, OperationError, OperationResult, TaskError};
use std::{
    future::{Future, poll_fn},
    io,
    pin::{Pin, pin},
    task::Poll,
};

/// Shared lifecycle support for task-backed stream writers. This is backend implementation support, not an application-facing writer.
pub struct SocketWriteState<H> {
    pending: Option<H>,
    cancellation: Cancellation,
    error: Option<OperationError>,
    closed: bool,
}

impl<H> Default for SocketWriteState<H> {
    fn default() -> Self {
        Self {
            pending: None,
            cancellation: Cancellation::new(),
            error: None,
            closed: false,
        }
    }
}

impl<H> SocketWriteState<H> {
    pub fn check_open(&self) -> OperationResult<()> {
        if self.closed {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "writer is closed").into());
        }
        self.result()
    }

    pub fn cancellation(&self) -> Cancellation {
        self.cancellation.clone()
    }

    pub fn accept(&mut self, pending: H) {
        debug_assert!(self.pending.is_none());
        self.pending = Some(pending);
    }

    pub fn fail(&mut self, error: OperationError) {
        if self.error.is_none() {
            self.error = Some(error);
        }
    }

    fn result(&self) -> OperationResult<()> {
        match &self.error {
            None => Ok(()),
            Some(OperationError::Cancelled(_)) => Err(crate::Cancelled.into()),
            Some(OperationError::Io(error)) => Err(match error.raw_os_error() {
                Some(code) => io::Error::from_raw_os_error(code),
                None => io::Error::new(error.kind(), error.to_string()),
            }
            .into()),
        }
    }
}

impl<H> SocketWriteState<H>
where
    H: Future<Output = Result<OperationResult<()>, TaskError>> + Unpin,
{
    async fn wait(&mut self, cancellation: &Cancellation) -> OperationResult<()> {
        if let Some(pending) = &mut self.pending {
            let mut cancelled = pin!(cancellation.cancelled());
            let outcome = poll_fn(|context| {
                if let Poll::Ready(result) = Pin::new(&mut *pending).poll(context) {
                    return Poll::Ready(Ok(result));
                }
                cancelled
                    .as_mut()
                    .poll(context)
                    .map(|()| Err(crate::Cancelled))
            })
            .await?;
            self.pending = None;
            match outcome {
                Ok(Ok(())) => {}
                Ok(Err(error)) => self.fail(error),
                Err(error) => {
                    let message = error.to_string();
                    self.fail(io::Error::other(message.clone()).into());
                    return Err(io::Error::other(message).into());
                }
            }
        }
        Ok(())
    }

    pub async fn flush(&mut self, cancellation: &Cancellation) -> OperationResult<()> {
        self.check_open()?;
        self.wait(cancellation).await?;
        self.result()
    }

    pub async fn close(&mut self) -> io::Result<()> {
        self.closed = true;
        self.cancellation.cancel();
        self.wait(&Cancellation::never()).await.map_err(Into::into)
    }
}

impl<H> Drop for SocketWriteState<H> {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

#[cfg(test)]
mod tests;
