// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::{
    BufferSlice, Cancellation, IoBuf, OperationResult, ReceiveBuffer, SocketReceiving,
    SocketRecvFlags, SocketRecvMessage, SocketRecvMsgReceiver,
};
use std::io;

/// A fixed-size example record: a retained range when contiguous, a copy when crossing receives.
pub enum Frame<B> {
    Shared(BufferSlice<B>),
    Copied(Vec<u8>),
}

impl<B: IoBuf> AsRef<[u8]> for Frame<B> {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Shared(buffer) => buffer.as_ref(),
            Self::Copied(buffer) => buffer,
        }
    }
}

/// A small framing example, not an HTTP parser. Only an unfinished record is copied between chunks.
pub struct FrameDecoder {
    size: usize,
    tail: Vec<u8>,
}

impl FrameDecoder {
    pub fn new(size: usize) -> io::Result<Self> {
        if size == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty frame"));
        }
        Ok(Self {
            size,
            tail: Vec::new(),
        })
    }

    pub fn push<B: IoBuf + Clone>(
        &mut self,
        buffer: B,
        mut consume: impl FnMut(Frame<B>) -> OperationResult<()>,
    ) -> OperationResult<()> {
        let bytes = buffer.as_ref();
        let mut offset = 0;
        if !self.tail.is_empty() {
            offset = (self.size - self.tail.len()).min(bytes.len());
            self.tail.extend_from_slice(&bytes[..offset]);
            if self.tail.len() != self.size {
                return Ok(());
            }
            consume(Frame::Copied(std::mem::take(&mut self.tail)))?;
        }
        while bytes.len() - offset >= self.size {
            consume(Frame::Shared(
                buffer.clone().slice(offset..offset + self.size),
            ))?;
            offset += self.size;
        }
        self.tail.extend_from_slice(&bytes[offset..]);
        Ok(())
    }

    pub fn finish(&self) -> io::Result<()> {
        if self.tail.is_empty() {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "incomplete frame",
            ))
        }
    }
}

/// Receive fixed-size records from a byte stream. The consumer may retain frames across receives; bound retention so at least one pool slot can progress. Datagram boundaries and truncation need a message-aware parser instead.
pub async fn receive<S, B, F>(
    scheduler: &S,
    socket: &S::Socket,
    pool: S::Pool,
    frame_size: usize,
    cancellation: &Cancellation,
    mut consume: F,
) -> OperationResult<()>
where
    S: SocketReceiving,
    B: ReceiveBuffer,
    S::PooledReceiver: SocketRecvMsgReceiver<S, Message: SocketRecvMessage<Payload = B>>,
    F: FnMut(Frame<B::Frozen>) -> OperationResult<()> + Send,
{
    let mut decoder = FrameDecoder::new(frame_size)?;
    let mut receiver = scheduler
        .socket_receiver(socket, pool, SocketRecvFlags::default(), cancellation)
        .await?;
    let result = async {
        while let Some(message) = receiver.recv(scheduler, cancellation).await? {
            decoder.push(message.into_payload().freeze(), &mut consume)?;
        }
        decoder.finish()?;
        Ok(())
    }
    .await;
    receiver.close(scheduler).await?;
    result
}
