// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Packet ECN: reporting each datagram's marking through every portable receive form, marking sent datagrams, and the options' gets and rejections.

use crate::{EcnOptionFixture, EcnRole, PacketEcnFixture};
use base_scheduler::*;
use std::io::{self, IoSliceMut};

/// The portable packet receive forms.
#[derive(Clone, Copy, Debug)]
enum Form {
    Read,
    ReadInto,
    ReadVectored,
    Peek,
    ReadVectoredWithFlags,
    ReadOwned,
    ReadOwnedVectored,
    PooledReader,
}

const FORMS: [Form; 8] = [
    Form::Read,
    Form::ReadInto,
    Form::ReadVectored,
    Form::Peek,
    Form::ReadVectoredWithFlags,
    Form::ReadOwned,
    Form::ReadOwnedVectored,
    Form::PooledReader,
];

fn never() -> Cancellation {
    Cancellation::never()
}

// The sockets' kernel options are those of the fixture; the readers are constructed when called.
struct Pair<'f, F: PacketEcnFixture> {
    fixture: &'f F,
}

impl<'f, F: PacketEcnFixture> Pair<'f, F>
where
    <F::Scheduler as SocketOperation>::Socket: 'static,
{
    fn scheduler(&self) -> &'f F::Scheduler {
        self.fixture.scheduler()
    }
    async fn set<O: SocketOption>(&self, role: EcnRole, option: O, value: O::Value)
    where
        F::Scheduler: SocketSetOption<O>,
    {
        self.scheduler()
            .socket_set_option(self.fixture.socket(role), option, value, &never())
            .await
            .unwrap();
    }
    async fn get<O: SocketOption>(&self, role: EcnRole, option: O) -> O::Value
    where
        F::Scheduler: SocketGetOption<O>,
    {
        self.scheduler()
            .socket_get_option(self.fixture.socket(role), option, &never())
            .await
            .unwrap()
    }
    async fn send(&self, payload: &[u8]) {
        let mut writer = self
            .scheduler()
            .socket_packet_writer(self.fixture.socket(EcnRole::Sender))
            .unwrap();
        let sent = writer
            .write(payload, Some(self.fixture.destination()), &never())
            .await
            .unwrap();
        assert_eq!(sent, payload.len());
    }
    // Send one datagram, then receive it through `form`, returning its reported marking.
    async fn round_trip(
        &self,
        reader: &mut impl SocketPacketReader,
        pooled: &mut impl SocketPacketReader,
        form: Form,
    ) -> Option<Ecn> {
        self.send(b"ecn").await;
        let never = never();
        let mut buffer = [0; 8];
        let result = match form {
            Form::Read => {
                let message = reader.read(&never).await.unwrap().unwrap();
                assert_eq!(message.payload(), b"ecn");
                return message.ecn();
            }
            Form::PooledReader => {
                let message = pooled.read(&never).await.unwrap().unwrap();
                assert_eq!(message.payload(), b"ecn");
                return message.ecn();
            }
            Form::ReadInto => reader.read_into(&mut buffer, &never).await,
            Form::ReadVectored => {
                reader
                    .read_vectored(&mut [IoSliceMut::new(&mut buffer)], &never)
                    .await
            }
            Form::Peek => {
                let peeked = reader
                    .read_into_with_flags(&mut buffer, SocketRecvFlags { peek: true }, &never)
                    .await
                    .unwrap();
                let consumed = reader.read_into(&mut buffer, &never).await.unwrap();
                assert_eq!(peeked.ecn, consumed.ecn, "a peek reported another marking");
                Ok(peeked)
            }
            Form::ReadVectoredWithFlags => {
                reader
                    .read_vectored_with_flags(
                        &mut [IoSliceMut::new(&mut buffer)],
                        SocketRecvFlags::default(),
                        &never,
                    )
                    .await
            }
            Form::ReadOwned => reader.read_owned(vec![0; 8], &never).await.0,
            Form::ReadOwnedVectored => {
                reader
                    .read_owned_vectored(
                        IoBuffersMut::new(vec![vec![0; 2], vec![0; 6]]),
                        SocketRecvFlags::default(),
                        &never,
                    )
                    .await
                    .0
            }
        };
        let result = result.unwrap();
        assert_eq!(result.bytes, 3);
        result.ecn
    }
    // The marking every form reports for one datagram each.
    async fn every_form(&self) -> Vec<(Form, Option<Ecn>)> {
        let scheduler = self.scheduler();
        let receiver = self.fixture.socket(EcnRole::Receiver);
        let mut reader = scheduler.socket_packet_reader(receiver).unwrap();
        let pool = scheduler
            .buffer_pool(BufferPoolOptions::new(2, 64))
            .unwrap();
        let mut pooled = scheduler
            .socket_packet_reader_with_pool(receiver, pool)
            .unwrap();
        let mut reports = Vec::new();
        for form in FORMS {
            reports.push((form, self.round_trip(&mut reader, &mut pooled, form).await));
        }
        reports
    }
}

fn assert_every(reports: Vec<(Form, Option<Ecn>)>, expected: Option<Ecn>) {
    for (form, ecn) in reports {
        assert_eq!(ecn, expected, "{form:?} reported the wrong marking");
    }
}

/// With `ReceiveEcn` enabled, a sender's `SendEcn` marking of ECT(0), and separately CE, is reported by every receive form, including peeks and owned and pooled receives.
pub async fn markings<F: PacketEcnFixture>(fixture: &F)
where
    <F::Scheduler as SocketOperation>::Socket: 'static,
{
    let pair = Pair { fixture };
    pair.set(EcnRole::Receiver, ReceiveEcn, true).await;
    for marking in [Ecn::Ect0, Ecn::Ce] {
        pair.set(EcnRole::Sender, SendEcn, marking).await;
        assert_every(pair.every_form().await, Some(marking));
    }
}

/// Reporting follows options set after a reader or pooled reader was constructed, in both directions.
pub async fn after_construction<F: PacketEcnFixture>(fixture: &F)
where
    <F::Scheduler as SocketOperation>::Socket: 'static,
{
    let pair = Pair { fixture };
    let scheduler = fixture.scheduler();
    let receiver = fixture.socket(EcnRole::Receiver);
    let mut reader = scheduler.socket_packet_reader(receiver).unwrap();
    let pool = scheduler
        .buffer_pool(BufferPoolOptions::new(2, 64))
        .unwrap();
    let mut pooled = scheduler
        .socket_packet_reader_with_pool(receiver, pool)
        .unwrap();
    pair.set(EcnRole::Sender, SendEcn, Ecn::Ect0).await;
    for (enabled, expected) in [(false, None), (true, Some(Ecn::Ect0)), (false, None)] {
        pair.set(EcnRole::Receiver, ReceiveEcn, enabled).await;
        for form in [Form::ReadInto, Form::Read, Form::PooledReader] {
            assert_eq!(
                pair.round_trip(&mut reader, &mut pooled, form).await,
                expected,
                "{form:?} did not follow ReceiveEcn set to {enabled}"
            );
        }
    }
}

/// With reporting disabled, every form reports `None` for a marked datagram; absence is never reported as `NotEct`.
pub async fn disabled<F: PacketEcnFixture>(fixture: &F)
where
    <F::Scheduler as SocketOperation>::Socket: 'static,
{
    let pair = Pair { fixture };
    pair.set(EcnRole::Sender, SendEcn, Ecn::Ect0).await;
    assert_every(pair.every_form().await, None);
}

/// Reporting enabled through the kernel option directly, not `ReceiveEcn`, is reported too.
pub async fn native_reporting<F: PacketEcnFixture>(fixture: &F)
where
    <F::Scheduler as SocketOperation>::Socket: 'static,
{
    let pair = Pair { fixture };
    fixture.set_native_receive(EcnRole::Receiver, true);
    pair.set(EcnRole::Sender, SendEcn, Ecn::Ce).await;
    assert_every(pair.every_form().await, Some(Ecn::Ce));
}

/// `SendEcn` preserves DSCP bits set beforehand.
pub async fn dscp<F: PacketEcnFixture>(fixture: &F)
where
    <F::Scheduler as SocketOperation>::Socket: 'static,
{
    let pair = Pair { fixture };
    fixture.set_native_class(EcnRole::Sender, 0xb8);
    pair.set(EcnRole::Sender, SendEcn, Ecn::Ect0).await;
    assert_eq!(fixture.native_class(EcnRole::Sender), 0xba);
    pair.set(EcnRole::Sender, SendEcn, Ecn::NotEct).await;
    assert_eq!(fixture.native_class(EcnRole::Sender), 0xb8);
}

/// Gets of both options return the kernel's values, including after native changes.
pub async fn gets<F: PacketEcnFixture>(fixture: &F)
where
    <F::Scheduler as SocketOperation>::Socket: 'static,
{
    let pair = Pair { fixture };
    let receiver = EcnRole::Receiver;
    assert!(
        std::ptr::eq(fixture.packet(), fixture.socket(receiver)),
        "the fixture's packet socket is not its receiver"
    );
    assert!(!pair.get(receiver, ReceiveEcn).await);
    pair.set(receiver, ReceiveEcn, true).await;
    assert!(pair.get(receiver, ReceiveEcn).await);
    fixture.set_native_receive(receiver, false);
    assert!(!pair.get(receiver, ReceiveEcn).await);
    fixture.set_native_receive(receiver, true);
    assert!(pair.get(receiver, ReceiveEcn).await);
    assert_eq!(pair.get(EcnRole::Sender, SendEcn).await, Ecn::NotEct);
    for (class, marking) in [(0x2b, Ecn::Ce), (0x01, Ecn::Ect1), (0xb8, Ecn::NotEct)] {
        fixture.set_native_class(EcnRole::Sender, class);
        assert_eq!(pair.get(EcnRole::Sender, SendEcn).await, marking);
    }
}

fn rejected<T: std::fmt::Debug>(result: OperationResult<T>, kind: io::ErrorKind) {
    match result {
        Err(OperationError::Io(error)) => assert_eq!(error.kind(), kind, "{error}"),
        other => panic!("expected {kind:?}, got {other:?}"),
    }
}

/// Stream sockets reject both options with `InvalidInput`, also where ECN is unsupported.
pub async fn streams<F: EcnOptionFixture>(fixture: &F) {
    let scheduler = fixture.scheduler();
    let stream = fixture.stream();
    let never = never();
    let kind = io::ErrorKind::InvalidInput;
    rejected(
        scheduler
            .socket_set_option(stream, ReceiveEcn, true, &never)
            .await,
        kind,
    );
    rejected(
        scheduler
            .socket_get_option(stream, ReceiveEcn, &never)
            .await,
        kind,
    );
    rejected(
        scheduler
            .socket_set_option(stream, SendEcn, Ecn::Ect0, &never)
            .await,
        kind,
    );
    rejected(
        scheduler.socket_get_option(stream, SendEcn, &never).await,
        kind,
    );
}

/// Without support, setting and getting either option on a packet socket returns `Unsupported`, never accepting a setting it cannot honour.
pub async fn unsupported<F: EcnOptionFixture>(fixture: &F) {
    let scheduler = fixture.scheduler();
    let socket = fixture.packet();
    let never = never();
    let kind = io::ErrorKind::Unsupported;
    rejected(
        scheduler
            .socket_set_option(socket, ReceiveEcn, true, &never)
            .await,
        kind,
    );
    rejected(
        scheduler
            .socket_get_option(socket, ReceiveEcn, &never)
            .await,
        kind,
    );
    rejected(
        scheduler
            .socket_set_option(socket, SendEcn, Ecn::Ect0, &never)
            .await,
        kind,
    );
    rejected(
        scheduler.socket_get_option(socket, SendEcn, &never).await,
        kind,
    );
}
