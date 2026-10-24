// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use core_scheduler::*;

macro_rules! conformance {
    ($($name:ident),*) => {$ (
        #[test]
        fn $name() {
            crate::test_support::run(|| {
                let scheduler = crate::test_support::scheduler();
                futures_lite::future::block_on(async {
                    let (sender, receiver) = core_scheduler_conformance::socket::stream_pair(&scheduler, SocketDomain::Ipv4).await;
                    core_scheduler_conformance::socket_stream_writing::$name(&scheduler, &sender, &receiver).await;
                    scheduler.shutdown().await;
                });
            });
        }
    )*};
}
conformance!(ordering, independent_progress, abandoned_admission);

#[test]
fn close_pending() {
    crate::test_support::run(|| {
        let scheduler = crate::test_support::scheduler();
        futures_lite::future::block_on(async {
            let (sender, _receiver) =
                core_scheduler_conformance::socket::stream_pair(&scheduler, SocketDomain::Ipv4)
                    .await;
            core_scheduler_conformance::socket_stream_writing::close_pending(&scheduler, &sender)
                .await;
            scheduler.shutdown().await;
        });
    });
}

#[test]
fn rejects_datagrams_and_closed_admission() {
    crate::test_support::run(|| {
        let scheduler = crate::test_support::scheduler();
        futures_lite::future::block_on(async {
            let never = Cancellation::never();
            let datagram = scheduler
                .socket_open(SocketDomain::Ipv4, SocketType::Datagram, None, &never)
                .await
                .unwrap();
            assert_eq!(
                scheduler
                    .socket_stream_writer::<Vec<u8>>(&datagram)
                    .err()
                    .unwrap()
                    .kind(),
                io::ErrorKind::InvalidInput
            );
            let (sender, _receiver) =
                core_scheduler_conformance::socket::stream_pair(&scheduler, SocketDomain::Ipv4)
                    .await;
            let mut writer = scheduler.socket_stream_writer::<Vec<u8>>(&sender).unwrap();
            scheduler.close();
            assert_eq!(
                scheduler
                    .socket_stream_writer::<Vec<u8>>(&sender)
                    .err()
                    .unwrap()
                    .kind(),
                io::ErrorKind::BrokenPipe
            );
            let (error, buffer) = writer
                .submit(&scheduler, vec![42], &never)
                .await
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
            assert_eq!(buffer, [42]);
            writer.close(&scheduler).await.unwrap();
            scheduler.shutdown().await;
        });
    });
}

#[test]
fn immediate_error_is_accepted_and_retained() {
    crate::test_support::run(|| {
        let scheduler = crate::test_support::scheduler();
        futures_lite::future::block_on(async {
            let never = Cancellation::never();
            let (sender, _receiver) =
                core_scheduler_conformance::socket::stream_pair(&scheduler, SocketDomain::Ipv4)
                    .await;
            scheduler
                .socket_shutdown(&sender, std::net::Shutdown::Write)
                .await
                .unwrap();
            let mut writer = scheduler.socket_stream_writer::<Vec<u8>>(&sender).unwrap();
            writer.submit(&scheduler, vec![42], &never).await.unwrap();
            let kind = writer.flush(&scheduler, &never).await.unwrap_err().kind();
            let (error, buffer) = writer
                .submit(&scheduler, vec![43], &never)
                .await
                .unwrap_err();
            assert_eq!(error.kind(), kind);
            assert_eq!(buffer, [43]);
            assert_eq!(
                writer.flush(&scheduler, &never).await.unwrap_err().kind(),
                kind
            );
            writer.close(&scheduler).await.unwrap();
            scheduler.shutdown().await;
        });
    });
}

#[test]
fn accepted_cancellation_and_drop_release_owners() {
    crate::test_support::run(|| {
        let scheduler = crate::test_support::scheduler();
        futures_lite::future::block_on(async {
            let never = Cancellation::never();
            let (sender, _receiver) =
                core_scheduler_conformance::socket::stream_pair(&scheduler, SocketDomain::Ipv4)
                    .await;
            let pool = HeapBufferPool::new(1, 8 * 1024 * 1024).unwrap();
            let mut writer = scheduler.socket_stream_writer(&sender).unwrap();
            let cancellation = Cancellation::new();
            writer
                .submit(
                    &scheduler,
                    pool.acquire(&never).await.unwrap(),
                    &cancellation,
                )
                .await
                .map_err(|(e, _)| e)
                .unwrap();
            cancellation.cancel();
            assert!(matches!(
                writer.flush(&scheduler, &never).await,
                Err(OperationError::Cancelled(_))
            ));
            assert!(pool.try_acquire().is_some());
            writer.close(&scheduler).await.unwrap();
            drop(writer);
            let mut writer = scheduler.socket_stream_writer(&sender).unwrap();
            writer
                .submit(&scheduler, pool.acquire(&never).await.unwrap(), &never)
                .await
                .map_err(|(e, _)| e)
                .unwrap();
            drop(writer);
            drop(pool.acquire(&never).await.unwrap());
            scheduler.shutdown().await;
        });
    });
}
