// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{
    IoUringBufferPool, IoUringRecvMessage, IoUringSelector, io_uring_receiver::IoUringReceiver,
    retained_socket::SocketSource,
};
use core_scheduler::{
    Cancellation, IoBufSequenceMut, OperationResult, SocketAddressView, SocketRecvFlags,
    SocketRecvMessage, SocketRecvResult,
};
use std::io::{self, IoSliceMut};

// Explicit provided pools can receive ahead. The retained message bridges callers switching to borrowed I/O without discarding stream suffixes or already queued datagrams.
#[derive(Default)]
pub(crate) struct ReceiveAhead {
    receiver: Option<IoUringReceiver>,
    pending: Option<IoUringRecvMessage>,
}
impl ReceiveAhead {
    pub(crate) fn active(&self) -> bool {
        self.receiver.is_some()
    }
    pub(crate) async fn read(
        &mut self,
        selector: &IoUringSelector,
        socket: &impl SocketSource,
        pool: &IoUringBufferPool,
        cancellation: &Cancellation,
    ) -> OperationResult<Option<IoUringRecvMessage>> {
        cancellation.check()?;
        if let Some(message) = self.pending.take() {
            return Ok(Some(message));
        }
        if self.receiver.is_none() {
            self.receiver = Some(
                selector
                    .socket_receiver(
                        socket,
                        pool.clone(),
                        SocketRecvFlags::default(),
                        cancellation,
                    )
                    .await?,
            );
        }
        self.receiver
            .as_mut()
            .unwrap()
            .recv(selector, cancellation)
            .await
    }
    async fn pending(
        &mut self,
        selector: &IoUringSelector,
        cancellation: &Cancellation,
    ) -> OperationResult<Option<&mut IoUringRecvMessage>> {
        if self.pending.is_none() {
            self.pending = self
                .receiver
                .as_mut()
                .expect("receive-ahead starts before buffered borrowing")
                .recv(selector, cancellation)
                .await?;
        }
        Ok(self.pending.as_mut())
    }
    pub(crate) async fn read_vectored(
        &mut self,
        selector: &IoUringSelector,
        buffers: &mut [IoSliceMut<'_>],
        flags: SocketRecvFlags,
        stream: bool,
        cancellation: &Cancellation,
    ) -> OperationResult<SocketRecvResult> {
        cancellation.check()?;
        crate::io_uring_selector::message::validate_lengths(
            buffers.len(),
            buffers.iter().map(|buffer| buffer.len()),
        )?;
        if stream && buffers.iter().all(|buffer| buffer.is_empty()) {
            return Ok(empty_result());
        }
        let Some(message) = self.pending(selector, cancellation).await? else {
            return Ok(empty_result());
        };
        let mut bytes = 0;
        for buffer in buffers {
            let count = buffer.len().min(message.payload().len() - bytes);
            buffer[..count].copy_from_slice(&message.payload()[bytes..bytes + count]);
            bytes += count;
        }
        let result = SocketRecvResult {
            bytes,
            address: message.address().map(|view| view.to_socket_address()),
            truncated: message.truncated() || (!stream && bytes < message.payload().len()),
            control_truncated: message.control_truncated(),
        };
        self.consume(bytes, flags.peek, stream);
        Ok(result)
    }
    pub(crate) async fn read_owned<Buffers: IoBufSequenceMut>(
        &mut self,
        selector: &IoUringSelector,
        mut buffers: Buffers,
        flags: SocketRecvFlags,
        stream: bool,
        cancellation: &Cancellation,
    ) -> (OperationResult<SocketRecvResult>, Buffers) {
        let result = async {
            cancellation.check()?;
            crate::io_uring_selector::message::validate_lengths(
                buffers.buffer_count(),
                (0..buffers.buffer_count()).map(|index| buffers.buffer(index).len()),
            )?;
            if stream && (0..buffers.buffer_count()).all(|index| buffers.buffer(index).is_empty()) {
                return Ok(empty_result());
            }
            let Some(message) = self.pending(selector, cancellation).await? else {
                return Ok(empty_result());
            };
            let mut bytes = 0;
            for index in 0..buffers.buffer_count() {
                let buffer = buffers.buffer_mut(index);
                let count = buffer.len().min(message.payload().len() - bytes);
                buffer[..count].copy_from_slice(&message.payload()[bytes..bytes + count]);
                bytes += count;
            }
            let result = SocketRecvResult {
                bytes,
                address: message.address().map(|view| view.to_socket_address()),
                truncated: message.truncated() || (!stream && bytes < message.payload().len()),
                control_truncated: message.control_truncated(),
            };
            self.consume(bytes, flags.peek, stream);
            Ok(result)
        }
        .await;
        (result, buffers)
    }
    fn consume(&mut self, bytes: usize, peek: bool, stream: bool) {
        if peek {
            return;
        }
        let message = self.pending.as_mut().unwrap();
        if !stream || bytes == message.payload().len() {
            self.pending = None;
        } else {
            message.advance_payload(bytes);
        }
    }
    pub(crate) async fn close(&mut self, selector: &IoUringSelector) -> io::Result<()> {
        if let Some(receiver) = &mut self.receiver {
            receiver.close(selector).await?;
        }
        self.pending = None;
        Ok(())
    }
}
fn empty_result() -> SocketRecvResult {
    SocketRecvResult {
        bytes: 0,
        address: None,
        truncated: false,
        control_truncated: false,
    }
}
