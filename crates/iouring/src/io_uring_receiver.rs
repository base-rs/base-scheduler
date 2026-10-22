// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{
    IoUringBuffer, IoUringBufferPool, IoUringMonitorRegistration, IoUringSelector, RecvMsgMultishot,
};
use core_scheduler::*;
use core_scheduler_selector::NativeSocket;
use std::io;

/// A demand-driven portable receiver or a worker-affine provided-buffer receive sequence. Pool construction selects the strategy once, without a per-message pool lookup.
pub enum IoUringReceiver {
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
    ) -> OperationResult<Option<RecvMsgMultishotMessage<Self::Buffer>>> {
        match self {
            Self::Demand(receiver) => receiver.recv(scheduler, cancellation).await,
            Self::Provided(receiver) => receiver.recv(scheduler, cancellation).await,
        }
    }
    async fn close(&mut self, scheduler: &IoUringSelector) -> io::Result<()> {
        match self {
            Self::Demand(receiver) => SocketRecvMsgReceiver::close(receiver, scheduler).await,
            Self::Provided(receiver) => receiver.close(scheduler).await,
        }
    }
}
