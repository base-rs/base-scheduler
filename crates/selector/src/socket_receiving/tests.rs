// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

#[test]
fn factory_checks_cancellation_socket_type_peeking_and_shutdown() {
    let scheduler = ReadinessSelector::new().unwrap();
    futures_lite::future::block_on(async {
        let never = Cancellation::never();
        let cancelled = Cancellation::new();
        cancelled.cancel();
        let socket = scheduler
            .socket_open(SocketDomain::Ipv4, SocketType::Stream, None, &never)
            .await
            .unwrap();
        assert!(
            scheduler
                .buffer_pool(BufferPoolOptions::new(0, 16))
                .is_err()
        );
        let pool = scheduler
            .buffer_pool(BufferPoolOptions::new(2, 16))
            .unwrap();
        assert!(
            scheduler
                .socket_receiver(
                    &socket,
                    pool.clone(),
                    SocketRecvFlags::default(),
                    &cancelled
                )
                .await
                .is_err()
        );
        crate::socket_io::SOCKET_TYPE_ERROR
            .with(|slot| slot.set(Some(std::io::ErrorKind::PermissionDenied)));
        let result = scheduler
            .socket_receiver(&socket, pool.clone(), SocketRecvFlags::default(), &never)
            .await;
        assert!(
            matches!(result, Err(OperationError::Io(error)) if error.kind() == std::io::ErrorKind::PermissionDenied)
        );
        assert!(
            scheduler
                .socket_receiver(
                    &socket,
                    pool.clone(),
                    SocketRecvFlags { peek: true },
                    &never
                )
                .await
                .is_err()
        );
        let receiver = scheduler
            .socket_receiver(&socket, pool.clone(), SocketRecvFlags::default(), &never)
            .await
            .unwrap();
        drop(receiver);
        scheduler.shutdown().await;
        assert!(
            scheduler
                .buffer_pool(BufferPoolOptions::new(2, 16))
                .is_err()
        );
        assert!(
            scheduler
                .socket_receiver(&socket, pool, SocketRecvFlags::default(), &never)
                .await
                .is_err()
        );
    });
}
