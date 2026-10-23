// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

#[test]
fn receiver_creation_is_lazy_and_close_checks_runtime_compatibility() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let foreign_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let scheduler = TokioScheduler::new(runtime.handle().clone());
    let foreign = TokioScheduler::new(foreign_runtime.handle().clone());
    runtime.block_on(async {
        let never = Cancellation::never();
        let (socket, peer) =
            core_scheduler_conformance::socket::stream_pair(&scheduler, SocketDomain::Ipv4).await;
        let pool = scheduler.buffer_pool(BufferPoolOptions::new(1, 8)).unwrap();
        let owners = std::sync::Arc::strong_count(&socket.inner);
        drop(scheduler.socket_receiver(&socket, pool.clone(), SocketRecvFlags::default(), &never));
        assert_eq!(std::sync::Arc::strong_count(&socket.inner), owners);
        let mut receiver = scheduler
            .socket_receiver(&socket, pool, SocketRecvFlags::default(), &never)
            .await
            .unwrap();
        assert_eq!(
            receiver.recv(&foreign, &never).await.err().unwrap().kind(),
            std::io::ErrorKind::InvalidInput
        );
        let (written, _) = scheduler.socket_write(&peer, vec![42], &never).await;
        assert_eq!(written.unwrap(), 1);
        let message = receiver.recv(&scheduler, &never).await.unwrap().unwrap();
        assert_eq!(message.payload(), &[42]);
        assert_eq!(
            receiver.close(&foreign).await.unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
        // A failed close may be retried with the owning scheduler, never used to resume reception.
        scheduler.close();
        receiver.close(&scheduler).await.unwrap();
        receiver.close(&scheduler).await.unwrap();
        drop(receiver);
        assert_eq!(std::sync::Arc::strong_count(&socket.inner), owners);
        // Ordinary admission is already closed. All monitor ownership is released, so the exclusive socket owners can now run their ordinary destructors.
        drop((socket, peer));
        assert_eq!(message.payload(), &[42]);
        scheduler.shutdown().await;
        foreign.shutdown().await;
    });
}

#[test]
fn factory_checks_cancellation_socket_type_peeking_and_shutdown() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let scheduler = TokioScheduler::new(runtime.handle().clone());
    runtime.block_on(async {
        let never = Cancellation::never();
        let cancelled = Cancellation::new();
        cancelled.cancel();
        let socket = scheduler.socket_open(SocketDomain::Ipv4, SocketType::Stream, None, &never).await.unwrap();
        assert!(scheduler.buffer_pool(BufferPoolOptions::new(0, 16)).is_err());
        let pool = scheduler.buffer_pool(BufferPoolOptions::new(2, 16)).unwrap();
        assert!(scheduler.socket_receiver(&socket, pool.clone(), SocketRecvFlags::default(), &cancelled).await.is_err());
        crate::socket_io::SOCKET_TYPE_ERROR.with(|slot| slot.set(Some(std::io::ErrorKind::PermissionDenied)));
        let result = scheduler.socket_receiver(&socket, pool.clone(), SocketRecvFlags::default(), &never).await;
        assert!(matches!(result, Err(OperationError::Io(error)) if error.kind() == std::io::ErrorKind::PermissionDenied));
        assert!(scheduler.socket_receiver(&socket, pool.clone(), SocketRecvFlags { peek: true }, &never).await.is_err());
        let receiver = scheduler.socket_receiver(&socket, pool.clone(), SocketRecvFlags::default(), &never).await.unwrap();
        drop(receiver);
        scheduler.shutdown().await;
        assert!(scheduler.buffer_pool(BufferPoolOptions::new(2, 16)).is_err());
        assert!(scheduler.socket_receiver(&socket, pool, SocketRecvFlags::default(), &never).await.is_err());
    });
}
