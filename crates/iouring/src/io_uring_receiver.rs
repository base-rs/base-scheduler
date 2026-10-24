// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::demand_receiver::DemandReceiver;
use crate::{IoUringBuffer, IoUringSelector, RecvMsgMultishot};
use core_scheduler::*;
use std::io;

/// An owned receive sequence whose buffer and I/O strategies are private to the backend. Pool construction selects the strategy once, without a per-message pool lookup.
pub(crate) struct IoUringReceiver(pub(crate) Strategy);

pub(crate) enum Strategy {
    Demand(DemandReceiver),
    Provided(RecvMsgMultishot<IoUringBuffer>),
}
impl IoUringReceiver {
    pub(crate) async fn recv(
        &mut self,
        scheduler: &IoUringSelector,
        cancellation: &Cancellation,
    ) -> OperationResult<Option<crate::IoUringRecvMessage>> {
        match &mut self.0 {
            Strategy::Demand(receiver) => receiver
                .recv(scheduler, cancellation)
                .await
                .map(|message| message.map(crate::IoUringRecvMessage::portable)),
            Strategy::Provided(receiver) => receiver.recv(cancellation).await,
        }
    }
    pub(crate) async fn close(&mut self, scheduler: &IoUringSelector) -> io::Result<()> {
        match &mut self.0 {
            Strategy::Demand(receiver) => receiver.close(scheduler).await,
            Strategy::Provided(receiver) => receiver.close().await,
        }
    }
}
