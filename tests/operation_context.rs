// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use futures_lite::future::block_on;
use protocol_scheduler::{
    BufferResult, Cancellation, IoBufMut, OperationError, SocketOperation, SocketRead,
};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

struct Body<Socket> {
    socket: Socket,
}

impl<Socket> Body<Socket> {
    async fn read<S: SocketRead<Socket = Socket>>(
        &self,
        scheduler: &S,
        cancellation: &Cancellation,
        buffer: Vec<u8>,
    ) -> BufferResult {
        scheduler
            .socket_read(&self.socket, buffer, cancellation)
            .await
    }
}

// Two test-only services intentionally share a resource representation. This
// establishes generic composition without promising compatibility for real backends.
struct ReadService {
    calls: AtomicUsize,
}

impl SocketOperation for ReadService {
    type Socket = Mutex<&'static [u8]>;
}

impl SocketRead for ReadService {
    async fn socket_read<B: IoBufMut>(
        &self,
        socket: &Self::Socket,
        mut buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        if let Err(error) = cancellation.check() {
            return (Err(error.into()), buffer);
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut input = socket.lock().unwrap();
        let count = input.len().min(buffer.as_mut().len());
        buffer.as_mut()[..count].copy_from_slice(&input[..count]);
        *input = &input[count..];
        (Ok(count), buffer)
    }
}

struct CompatibleService(ReadService);

impl SocketOperation for CompatibleService {
    type Socket = <ReadService as SocketOperation>::Socket;
}

impl SocketRead for CompatibleService {
    async fn socket_read<B: IoBufMut>(
        &self,
        socket: &Self::Socket,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.0.socket_read(socket, buffer, cancellation).await
    }
}

#[test]
fn a_body_selects_execution_context_for_each_operation() {
    let first = ReadService {
        calls: AtomicUsize::new(0),
    };
    let second = ReadService {
        calls: AtomicUsize::new(0),
    };
    let third = CompatibleService(ReadService {
        calls: AtomicUsize::new(0),
    });
    let body = Body {
        socket: Mutex::new(b"abcdef".as_slice()),
    };
    let token = Cancellation::new();
    let buffer = vec![0; 2];
    let allocation = buffer.as_ptr() as usize;

    let unpolled = body.read(&first, &token, vec![0; 2]);
    drop(unpolled);
    assert_eq!(first.calls.load(Ordering::SeqCst), 0);
    assert_eq!(*body.socket.lock().unwrap(), b"abcdef");

    let wait = body.read(&first, &token, buffer);
    // Moving a Send future to a different polling thread retains its explicit context.
    let (result, buffer) =
        std::thread::scope(|scope| scope.spawn(move || block_on(wait)).join().unwrap());
    assert_eq!(result.unwrap(), 2);
    assert_eq!(buffer, b"ab");
    assert_eq!(buffer.as_ptr() as usize, allocation);

    token.cancel();
    let (result, buffer) = block_on(body.read(&second, &token, buffer));
    assert!(matches!(result, Err(OperationError::Cancelled(_))));
    assert_eq!(second.calls.load(Ordering::SeqCst), 0);

    let fresh = Cancellation::new();
    let (result, buffer) = block_on(body.read(&second, &fresh, buffer));
    assert_eq!(result.unwrap(), 2);
    assert_eq!(buffer, b"cd");
    let (result, buffer) = block_on(body.read(&third, &Cancellation::never(), buffer));
    assert_eq!(result.unwrap(), 2);
    assert_eq!(buffer, b"ef");
    assert_eq!(buffer.as_ptr() as usize, allocation);
    assert_eq!(first.calls.load(Ordering::SeqCst), 1);
    assert_eq!(second.calls.load(Ordering::SeqCst), 1);
    assert_eq!(third.0.calls.load(Ordering::SeqCst), 1);
}
