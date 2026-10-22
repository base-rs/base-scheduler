// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::io_uring_selector::tests::{BEFORE_SYSCALL, Failures, pair};
use futures_lite::future::block_on;

#[test]
fn factory_preserves_admission_errors_and_socket_type_failure() {
    let (selector, _driver) = pair(2);
    let never = Cancellation::never();
    let cancelled = Cancellation::new();
    cancelled.cancel();
    let socket =
        block_on(selector.socket_open(SocketDomain::Ipv4, SocketType::Stream, None, &never))
            .unwrap();
    assert!(selector.buffer_pool(BufferPoolOptions::new(0, 16)).is_err());
    let pool = selector.buffer_pool(BufferPoolOptions::new(2, 16)).unwrap();
    assert!(
        block_on(selector.socket_receiver(
            &socket,
            pool.clone(),
            SocketRecvFlags::default(),
            &cancelled
        ))
        .is_err()
    );
    let _failures = Failures::new([("socket-type", io::ErrorKind::PermissionDenied.into())]);
    assert!(
        matches!(block_on(selector.socket_receiver(&socket, pool.clone(), SocketRecvFlags::default(), &never)), Err(OperationError::Io(error)) if error.kind() == io::ErrorKind::PermissionDenied)
    );
    assert!(
        block_on(selector.socket_receiver(
            &socket,
            pool.clone(),
            SocketRecvFlags { peek: true },
            &never
        ))
        .is_err()
    );
    let group = Arc::clone(&selector.group);
    BEFORE_SYSCALL
        .with(|hook| *hook.borrow_mut() = Some(("socket-type", Box::new(move || group.close()))));
    assert!(
        block_on(selector.socket_receiver(
            &socket,
            pool.clone(),
            SocketRecvFlags::default(),
            &never
        ))
        .is_err()
    );
    selector.close();
    assert!(selector.buffer_pool(BufferPoolOptions::new(2, 16)).is_err());
    assert!(
        block_on(selector.socket_receiver(&socket, pool, SocketRecvFlags::default(), &never))
            .is_err()
    );
}
