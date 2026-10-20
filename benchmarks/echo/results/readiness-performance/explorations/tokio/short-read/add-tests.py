from pathlib import Path
p=Path('/home/samuel/Developer/core-rs/core-scheduler-perf-tokio-next/crates/tokio/src/tokio_socket/tests.rs')
s=p.read_text();s+='''
#[cfg(target_os = "linux")]
#[test]
fn short_stream_reads_do_not_retry_stale_readiness() {
    use core_scheduler::*;
    use futures_lite::future::{block_on, poll_once};
    use std::io::Write;
    crate::test_support::run(|| {
        block_on(async {
            for borrowed in [false, true] {
                for concurrent in [false, true] {
                    let scheduler = crate::test_support::scheduler();
                    let never = Cancellation::never();
                    let (native, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
                    let socket = TokioSocket::new(&scheduler.runtime, native.into()).unwrap();
                    // A real pending operation reserves the single-waker slot. The second reader must use the independent waiter path.
                    let mut reserved = Box::pin(socket.io(Interest::Readable, |_| {
                        Err::<(), _>(io::ErrorKind::WouldBlock.into())
                    }));
                    if concurrent {
                        assert!(poll_once(&mut reserved).await.is_none());
                    }
                    let mut buffer = [0; 8];
                    let mut receiving = Box::pin(async {
                        if borrowed {
                            scheduler.socket_recv_poll(&socket, &mut buffer, SocketRecvFlags::default(), &never).await.unwrap()
                        } else {
                            let (result, received) = scheduler.socket_read(&socket, vec![0; 8], &never).await;
                            buffer.copy_from_slice(&received);
                            result.unwrap()
                        }
                    });
                    assert!(poll_once(&mut receiving).await.is_none());
                    peer.write_all(b"x").unwrap();
                    assert_eq!(receiving.await, 1);
                    assert_eq!(buffer[0], b'x');
                    let mut attempts = 0;
                    let mut next = Box::pin(socket.io(Interest::Readable, |socket| {
                        attempts += 1;
                        crate::socket_io::recv(socket, &mut buffer)
                    }));
                    assert!(poll_once(&mut next).await.is_none());
                    drop(next);
                    assert_eq!(attempts, 1, "a drained stream must not retry a stale readable event");
                    peer.write_all(b"y").unwrap();
                    assert_eq!(scheduler.socket_recv_poll(&socket, &mut buffer, SocketRecvFlags::default(), &never).await.unwrap(), 1);
                    assert_eq!(buffer[0], b'y');
                    drop(reserved);
                    scheduler.socket_close(socket).await.unwrap();
                    scheduler.shutdown().await;
                }
            }
        });
    });
}

#[cfg(target_os = "linux")]
#[test]
fn full_peeking_and_datagram_reads_retain_readability() {
    use core_scheduler::*;
    use futures_lite::future::{block_on, poll_once};
    use std::io::Write;
    crate::test_support::run(|| {
        block_on(async {
            let scheduler = crate::test_support::scheduler();
            let never = Cancellation::never();
            for peek in [false, true] {
                let (native, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
                let socket = TokioSocket::new(&scheduler.runtime, native.into()).unwrap();
                // A full read leaves bytes queued; a short peek leaves all bytes queued.
                let mut buffer = vec![0; if peek { 8 } else { 1 }];
                let mut receiving = Box::pin(scheduler.socket_recv_poll(&socket, &mut buffer, SocketRecvFlags { peek }, &never));
                assert!(poll_once(&mut receiving).await.is_none());
                peer.write_all(b"xy").unwrap();
                assert_eq!(receiving.await.unwrap(), if peek { 2 } else { 1 });
                assert!(poll_once(socket.inner.registration.get().unwrap().readable()).await.is_some());
                let mut rest = [0; 8];
                assert_eq!(scheduler.socket_recv_poll(&socket, &mut rest, SocketRecvFlags::default(), &never).await.unwrap(), if peek { 2 } else { 1 });
                assert_eq!(rest[0], if peek { b'x' } else { b'y' });
                scheduler.socket_close(socket).await.unwrap();
            }
            let native = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            let address = native.local_addr().unwrap();
            let peer = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            let socket = TokioSocket::new(&scheduler.runtime, native.into()).unwrap();
            let mut buffer = [0; 8];
            let mut receiving = Box::pin(scheduler.socket_recv_poll(&socket, &mut buffer, SocketRecvFlags::default(), &never));
            assert!(poll_once(&mut receiving).await.is_none());
            peer.send_to(b"x", address).unwrap();
            peer.send_to(b"y", address).unwrap();
            assert_eq!(receiving.await.unwrap(), 1);
            assert!(poll_once(socket.inner.registration.get().unwrap().readable()).await.is_some());
            assert_eq!(scheduler.socket_recv_poll(&socket, &mut buffer, SocketRecvFlags::default(), &never).await.unwrap(), 1);
            assert_eq!(buffer[0], b'y');
            scheduler.socket_close(socket).await.unwrap();
            scheduler.shutdown().await;
        });
    });
}
''';p.write_text(s)
