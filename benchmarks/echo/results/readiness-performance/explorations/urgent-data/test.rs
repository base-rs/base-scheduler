
#[cfg(target_os = "linux")]
#[test]
fn urgent_tcp_boundary_keeps_trailing_bytes_readable() {
    use core_scheduler::*;
    use futures_lite::future::block_on;
    use std::io::Write;
    let runtime = runtime();
    let scheduler = crate::TokioScheduler::with_options(runtime.handle().clone(), crate::TokioOptions { threads: 1, capacity: 4 }).unwrap();
    let never = Cancellation::never();
    let (native, mut peer) = tcp_pair();
    let socket = TokioSocket::new(runtime.handle(), native.into()).unwrap();
    let registration = block_on(socket.inner.registration()).unwrap();
    let mut buffer = [0; 64];
    peer.write_all(b"seed").unwrap();
    assert_eq!(block_on(scheduler.socket_recv_poll(&socket, &mut buffer, SocketRecvFlags::default(), &never)).unwrap(), 4);
    assert!(socket.inner.connected_stream.load(Ordering::Relaxed));
    peer.write_all(b"before").unwrap();
    let peer_socket = socket2::SockRef::from(&peer);
    assert_eq!(peer_socket.send_with_flags(b"!", libc::MSG_OOB).unwrap(), 1);
    peer.write_all(b"after").unwrap();
    // Let every loopback segment arrive before the current-thread reactor captures its edge.
    std::thread::sleep(Duration::from_millis(10));
    runtime.block_on(async { registration.readable().await.unwrap(); });
    let count = runtime.block_on(scheduler.socket_recv_poll(&socket, &mut buffer, SocketRecvFlags::default(), &never)).unwrap();
    assert_eq!(&buffer[..count], b"before");
    let result = runtime.block_on(async {
        tokio::time::timeout(Duration::from_millis(100), scheduler.socket_recv_poll(&socket, &mut buffer, SocketRecvFlags::default(), &never)).await
    });
    // The first read stops at the urgent mark. The following ordinary bytes are already queued; another peer write must not be necessary.
    assert!(result.is_ok(), "trailing normal TCP bytes stalled after a short read at an urgent mark");
    let count = result.unwrap().unwrap();
    assert_eq!(&buffer[..count], b"after");
    block_on(scheduler.socket_close(socket)).unwrap();
    block_on(scheduler.shutdown());
}
