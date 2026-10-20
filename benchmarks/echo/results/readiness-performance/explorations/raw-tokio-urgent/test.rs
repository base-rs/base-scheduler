
#[cfg(target_os = "linux")]
#[test]
fn raw_tokio_urgent_boundary_comparison() {
    use std::io::Write;
    use tokio::io::{AsyncRead, ReadBuf};
    async fn read(socket: &mut tokio::net::TcpStream, buffer: &mut [u8]) -> io::Result<usize> {
        let mut buffer = ReadBuf::new(buffer);
        std::future::poll_fn(|context| std::pin::Pin::new(&mut *socket).poll_read(context, &mut buffer)).await?;
        Ok(buffer.filled().len())
    }
    for direct in [false, true] {
        let runtime = runtime();
        let (native, mut peer) = tcp_pair();
        native.set_nonblocking(true).unwrap();
        let mut socket = {
            let _scope = runtime.enter();
            tokio::net::TcpStream::from_std(native).unwrap()
        };
        let mut buffer = [0; 64];
        peer.write_all(b"seed").unwrap();
        assert_eq!(runtime.block_on(read(&mut socket, &mut buffer)).unwrap(), 4);
        peer.write_all(b"before").unwrap();
        let peer_socket = socket2::SockRef::from(&peer);
        assert_eq!(peer_socket.send_with_flags(b"!", libc::MSG_OOB).unwrap(), 1);
        peer.write_all(b"after").unwrap();
        // Queue every loopback segment before this current-thread reactor captures the edge.
        std::thread::sleep(Duration::from_millis(10));
        runtime.block_on(socket.readable()).unwrap();
        let first = if direct {
            socket.try_read(&mut buffer).unwrap()
        } else {
            runtime.block_on(read(&mut socket, &mut buffer)).unwrap()
        };
        assert_eq!(&buffer[..first], b"before");
        let second = runtime.block_on(async {
            tokio::time::timeout(Duration::from_millis(100), async {
                if direct {
                    loop {
                        socket.readable().await.unwrap();
                        match socket.try_read(&mut buffer) {
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => (),
                            result => break result,
                        }
                    }
                } else {
                    read(&mut socket, &mut buffer).await
                }
            }).await
        });
        if direct {
            let count = second.expect("try_read control stalled").unwrap();
            assert_eq!(&buffer[..count], b"after");
            println!("raw Tokio readable+try_read: before then after, no timeout");
        } else {
            assert!(second.is_err(), "upstream AsyncRead did not reproduce the short-read stall");
            // The ordinary payload is still immediately available through a direct syscall.
            use std::os::fd::AsRawFd;
            let count = unsafe { libc::recv(socket.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len(), libc::MSG_DONTWAIT) };
            assert_eq!(count, 5);
            assert_eq!(&buffer[..count as usize], b"after");
            println!("raw Tokio AsyncRead::poll_read: before then 100ms timeout; direct recv returned after");
        }
    }
}
