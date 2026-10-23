// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{
    IoUringBuffer, IoUringBufferPool, IoUringMonitorRegistration, IoUringSelector, RecvMsgMultishot,
};
use core_scheduler::*;
use core_scheduler_selector::NativeSocket;
use std::io;

/// An owned receive sequence whose buffer and I/O strategies are private to the backend. Pool construction selects the strategy once, without a per-message pool lookup.
pub struct IoUringReceiver(pub(crate) Strategy);

pub(crate) enum Strategy {
    Demand(
        SocketReceiver<SocketMonitor<NativeSocket, IoUringMonitorRegistration>, IoUringBufferPool>,
    ),
    Provided(RecvMsgMultishot<IoUringBuffer>),
}
impl SocketRecvMsgReceiver<IoUringSelector> for IoUringReceiver {
    type Buffer = IoUringBuffer;
    async fn recv(
        &mut self,
        scheduler: &IoUringSelector,
        cancellation: &Cancellation,
    ) -> OperationResult<Option<SocketRecvMessage<Self::Buffer>>> {
        match &mut self.0 {
            Strategy::Demand(receiver) => receiver.recv(scheduler, cancellation).await,
            Strategy::Provided(receiver) => receiver.recv(scheduler, cancellation).await,
        }
    }
    async fn close(&mut self, scheduler: &IoUringSelector) -> io::Result<()> {
        match &mut self.0 {
            Strategy::Demand(receiver) => SocketRecvMsgReceiver::close(receiver, scheduler).await,
            Strategy::Provided(receiver) => receiver.close(scheduler).await,
        }
    }
}
