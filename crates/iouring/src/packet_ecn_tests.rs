// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

// Packet ECN on the multishot experiment: its reporting and payload capacity, over loopback. `tests/packet_ecn.rs` runs the conformance suite.

use crate::{IoUringOptions, IoUringSelector, RecvMsgMultishotOptions};
use base_scheduler::{
    Cancellation, Ecn, ReceiveEcn, SendEcn, SocketRecvFlags, SocketRecvMessage, SocketSetOption,
};
use base_scheduler_conformance::{
    EcnFamily, EcnOptionFixture, EcnRole, LoopbackEcnFixture, PacketEcnFixture,
};
use futures_lite::future::block_on;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::mpsc,
    time::Duration,
};

fn run(
    family: EcnFamily,
    test: impl AsyncFnOnce(&LoopbackEcnFixture<'_, IoUringSelector>) + Send + 'static,
) {
    let (done, completed) = mpsc::channel();
    std::thread::spawn(move || {
        let result = catch_unwind(AssertUnwindSafe(|| {
            let (selector, driver) = IoUringSelector::new(IoUringOptions {
                entries: 16,
                ..Default::default()
            })
            .unwrap();
            let thread = std::thread::spawn(move || driver.run());
            let fixture = block_on(LoopbackEcnFixture::open(&selector, family));
            let result = catch_unwind(AssertUnwindSafe(|| block_on(test(&fixture))));
            block_on(fixture.close());
            block_on(selector.shutdown());
            thread.join().unwrap().unwrap();
            if let Err(panic) = result {
                std::panic::resume_unwind(panic);
            }
        }));
        let _ = done.send(result);
    });
    if let Err(panic) = completed
        .recv_timeout(Duration::from_secs(20))
        .expect("packet ECN test timed out")
    {
        std::panic::resume_unwind(panic);
    }
}

// A multishot receiver reports the marking while reporting is enabled, including when it was enabled after the receiver was constructed, and none while disabled. Control storage does not reduce the configured payload capacity.
#[test]
fn multishot_reports_the_marking_with_unchanged_capacity() {
    for family in [EcnFamily::Ipv4, EcnFamily::Ipv6, EcnFamily::DualStack] {
        run(family, async |fixture| {
            let selector = fixture.scheduler();
            let never = Cancellation::never();
            let receiver = fixture.socket(EcnRole::Receiver);
            selector
                .socket_set_option(fixture.socket(EcnRole::Sender), SendEcn, Ecn::Ect0, &never)
                .await
                .unwrap();
            let mut receiving = selector
                .socket_recvmsg_multishot(
                    receiver,
                    RecvMsgMultishotOptions {
                        buffers: 4,
                        buffer_size: 8,
                    },
                    SocketRecvFlags::default(),
                    &never,
                )
                .await
                .unwrap();
            let sender = fixture.socket(EcnRole::Sender);
            let send = async |payload: &[u8]| {
                let mut writer =
                    base_scheduler::SocketPacketWriting::socket_packet_writer(selector, sender)
                        .unwrap();
                base_scheduler::SocketPacketWriter::write(
                    &mut writer,
                    payload,
                    Some(fixture.destination()),
                    &never,
                )
                .await
                .unwrap();
            };
            for (enabled, expected) in [(false, None), (true, Some(Ecn::Ect0)), (false, None)] {
                selector
                    .socket_set_option(receiver, ReceiveEcn, enabled, &never)
                    .await
                    .unwrap();
                send(b"12345678").await;
                let message = receiving.recv(&never).await.unwrap().unwrap();
                assert_eq!(message.payload(), b"12345678");
                assert!(!message.truncated());
                assert_eq!(message.ecn(), expected);
                send(b"123456789").await;
                let message = receiving.recv(&never).await.unwrap().unwrap();
                assert_eq!(message.payload(), b"12345678");
                assert!(message.truncated());
                assert_eq!(message.ecn(), expected);
            }
            receiving.close().await.unwrap();
        });
    }
}
