// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

// A controlled option-only backend without ECN support: the unsupported suite accepts it, and rejects faults that accept a setting it cannot honour or a stream socket.

mod support;
use base_scheduler::*;
use base_scheduler_conformance::{EcnOptionFixture, packet_ecn};
use futures_lite::future::block_on;
use std::io;

#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    AcceptPacket,
    WrongError,
    AcceptStream,
}

#[derive(Clone, Copy, PartialEq)]
enum Socket {
    Packet,
    Stream,
}

struct Unsupported(Fault);

impl Unsupported {
    fn result<T: Default>(&self, socket: &Socket) -> OperationResult<T> {
        match (socket, self.0) {
            (Socket::Stream, Fault::AcceptStream) | (Socket::Packet, Fault::AcceptPacket) => {
                Ok(T::default())
            }
            (Socket::Stream, _) => Err(io::Error::from(io::ErrorKind::InvalidInput).into()),
            (Socket::Packet, Fault::WrongError) => {
                Err(io::Error::from(io::ErrorKind::InvalidInput).into())
            }
            (Socket::Packet, _) => Err(io::Error::from(io::ErrorKind::Unsupported).into()),
        }
    }
}

impl SocketOperation for Unsupported {
    type Socket = Socket;
}

impl SocketGetOption<ReceiveEcn> for Unsupported {
    async fn socket_get_option(
        &self,
        socket: &Socket,
        _: ReceiveEcn,
        _: &Cancellation,
    ) -> OperationResult<bool> {
        self.result(socket)
    }
}

impl SocketSetOption<ReceiveEcn> for Unsupported {
    async fn socket_set_option(
        &self,
        socket: &Socket,
        _: ReceiveEcn,
        _: bool,
        _: &Cancellation,
    ) -> OperationResult<()> {
        self.result(socket)
    }
}

impl SocketGetOption<SendEcn> for Unsupported {
    async fn socket_get_option(
        &self,
        socket: &Socket,
        _: SendEcn,
        _: &Cancellation,
    ) -> OperationResult<Ecn> {
        self.result::<()>(socket).map(|()| Ecn::NotEct)
    }
}

impl SocketSetOption<SendEcn> for Unsupported {
    async fn socket_set_option(
        &self,
        socket: &Socket,
        _: SendEcn,
        _: Ecn,
        _: &Cancellation,
    ) -> OperationResult<()> {
        self.result(socket)
    }
}

struct Fixture(Unsupported);

impl EcnOptionFixture for Fixture {
    type Scheduler = Unsupported;
    fn scheduler(&self) -> &Unsupported {
        &self.0
    }
    fn packet(&self) -> &Socket {
        &Socket::Packet
    }
    fn stream(&self) -> &Socket {
        &Socket::Stream
    }
}

fn run(test: impl AsyncFnOnce(&Fixture) + Send + 'static) {
    support::run(move || block_on(test(&Fixture(Unsupported(Fault::None)))));
}

mod unsupported {
    base_scheduler_conformance::packet_ecn_unsupported_tests!(run: super::run);
}

fn fails(fault: Fault, case: fn(&Fixture) -> std::pin::Pin<Box<dyn Future<Output = ()> + '_>>) {
    let fixture = Fixture(Unsupported(fault));
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| block_on(case(&fixture))));
    assert!(result.is_err(), "the suite accepted a faulty backend");
}

#[test]
fn rejects_a_backend_that_accepts_an_unsupported_setting() {
    fails(Fault::AcceptPacket, |fixture| {
        Box::pin(packet_ecn::unsupported(fixture))
    });
}

#[test]
fn rejects_a_backend_that_reports_the_wrong_error() {
    fails(Fault::WrongError, |fixture| {
        Box::pin(packet_ecn::unsupported(fixture))
    });
}

#[test]
fn rejects_a_backend_that_accepts_a_stream_socket() {
    fails(Fault::AcceptStream, |fixture| {
        Box::pin(packet_ecn::streams(fixture))
    });
}
