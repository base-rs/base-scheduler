// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Reusable directional monitor assertions, including caller-owned connection storage.
use core_scheduler::*;
use futures_lite::future::{poll_once, zip};
use std::{
    io::{IoSlice, IoSliceMut},
    sync::Arc,
};

/// Exercise separate reader/writer state, reuse, borrowed message operations, cancellation, and direction validation on an isolated stream pair.
pub async fn stream<S: SocketMonitoring>(scheduler: &S, sender: &S::Socket, receiver: &S::Socket)
where
    S::Socket: 'static,
{
    monitor(
        scheduler,
        sender,
        receiver,
        |socket, interest| scheduler.socket_monitor_state(socket, interest).unwrap(),
        |socket, interest| scheduler.socket_monitor(socket, interest).unwrap(),
    )
    .await;
    monitor(
        scheduler,
        sender,
        receiver,
        |socket, interest| {
            scheduler
                .socket_stream_monitor_state(socket, interest)
                .unwrap()
        },
        |socket, interest| scheduler.socket_stream_monitor(socket, interest).unwrap(),
    )
    .await;
}

async fn monitor<S, M>(
    scheduler: &S,
    sender: &S::Socket,
    receiver: &S::Socket,
    state: impl Fn(&S::Socket, Interest) -> M,
    storage: impl Fn(&S::Socket, Interest) -> MonitorStorage<M>,
) where
    S: SocketMonitoring,
    S::Socket: 'static,
    M: SocketMonitorState<S::Socket, S::MonitorRegistration>,
{
    let never = Cancellation::never();
    struct Connection<M> {
        read: M,
        write: M,
    }
    let connection = Arc::new(Connection {
        read: state(receiver, Interest::Readable),
        write: state(receiver, Interest::Writable),
    });
    let mut reader =
        MonitorStorage::from_arc(Arc::clone(&connection), |connection| &connection.read);
    let mut writer =
        MonitorStorage::from_arc(Arc::clone(&connection), |connection| &connection.write);
    let pointer = reader.as_ptr();
    drop(connection);
    let mut sender_write = storage(sender, Interest::Writable);
    let mut sender_read = storage(sender, Interest::Readable);
    for byte in *b"ab" {
        let mut data = [0];
        let (read, write) = zip(
            scheduler.socket_monitor_recv_poll(
                &mut reader,
                &mut data,
                SocketRecvFlags::default(),
                &never,
            ),
            scheduler.socket_monitor_send_poll(
                &mut sender_write,
                &[byte],
                SocketSendFlags::default(),
                &never,
            ),
        )
        .await;
        assert_eq!(read.unwrap(), 1);
        assert_eq!(write.unwrap(), 1);
        assert_eq!(data, [byte]);
        assert_eq!(reader.as_ptr(), pointer);
        scheduler
            .socket_monitor_wait(&mut writer, &never)
            .await
            .unwrap();
        let mut returned = [0];
        let mut buffers = [IoSliceMut::new(&mut returned)];
        let vectors = [IoSlice::new(&data)];
        let (read, write) = zip(
            scheduler.socket_monitor_recvmsg_poll(
                &mut sender_read,
                &mut buffers,
                SocketRecvFlags::default(),
                &never,
            ),
            scheduler.socket_monitor_sendmsg_poll(
                &mut writer,
                &vectors,
                None,
                SocketSendFlags::default(),
                &never,
            ),
        )
        .await;
        assert_eq!(read.unwrap().bytes, 1);
        assert_eq!(write.unwrap(), 1);
        assert_eq!(returned, [byte]);
    }
    // A reader and writer on the same socket must progress independently.
    let mut incoming = [0];
    let mut outgoing = [0];
    let (server, client) = zip(
        zip(
            scheduler.socket_monitor_recv_poll(
                &mut reader,
                &mut incoming,
                SocketRecvFlags::default(),
                &never,
            ),
            scheduler.socket_monitor_send_poll(
                &mut writer,
                b"s",
                SocketSendFlags::default(),
                &never,
            ),
        ),
        zip(
            scheduler.socket_monitor_send_poll(
                &mut sender_write,
                b"c",
                SocketSendFlags::default(),
                &never,
            ),
            scheduler.socket_monitor_recv_poll(
                &mut sender_read,
                &mut outgoing,
                SocketRecvFlags::default(),
                &never,
            ),
        ),
    )
    .await;
    assert_eq!(server.0.unwrap(), 1);
    assert_eq!(server.1.unwrap(), 1);
    assert_eq!(client.0.unwrap(), 1);
    assert_eq!(client.1.unwrap(), 1);
    assert_eq!(incoming, *b"c");
    assert_eq!(outgoing, *b"s");
    let cancellation = Cancellation::new();
    let mut data = [0];
    let mut waiting = Box::pin(scheduler.socket_monitor_recv_poll(
        &mut reader,
        &mut data,
        SocketRecvFlags::default(),
        &cancellation,
    ));
    assert!(poll_once(waiting.as_mut()).await.is_none());
    cancellation.cancel();
    cancelled(waiting.await);
    // Cancellation before polling must not submit work or inspect the payload.
    cancelled(
        scheduler
            .socket_monitor_wait(&mut reader, &cancellation)
            .await,
    );
    cancelled(
        scheduler
            .socket_monitor_recv_poll(
                &mut reader,
                &mut data,
                SocketRecvFlags::default(),
                &cancellation,
            )
            .await,
    );
    cancelled(
        scheduler
            .socket_monitor_send_poll(
                &mut writer,
                &data,
                SocketSendFlags::default(),
                &cancellation,
            )
            .await,
    );
    let mut buffers = [IoSliceMut::new(&mut data)];
    cancelled(
        scheduler
            .socket_monitor_recvmsg_poll(
                &mut reader,
                &mut buffers,
                SocketRecvFlags::default(),
                &cancellation,
            )
            .await,
    );
    let vectors = [IoSlice::new(b"x")];
    cancelled(
        scheduler
            .socket_monitor_sendmsg_poll(
                &mut writer,
                &vectors,
                None,
                SocketSendFlags::default(),
                &cancellation,
            )
            .await,
    );
    assert_eq!(
        scheduler
            .socket_monitor_recv_poll(&mut writer, &mut data, SocketRecvFlags::default(), &never)
            .await
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert_eq!(
        scheduler
            .socket_monitor_send_poll(&mut reader, &data, SocketSendFlags::default(), &never)
            .await
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidInput
    );
    let mut buffers = [IoSliceMut::new(&mut data)];
    assert_eq!(
        scheduler
            .socket_monitor_recvmsg_poll(
                &mut writer,
                &mut buffers,
                SocketRecvFlags::default(),
                &never
            )
            .await
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert_eq!(
        scheduler
            .socket_monitor_sendmsg_poll(
                &mut reader,
                &vectors,
                None,
                SocketSendFlags::default(),
                &never
            )
            .await
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidInput
    );
    // Drop a pending read, then reuse the monitor while its former registration retires.
    let mut pending = Box::pin(scheduler.socket_monitor_recv_poll(
        &mut reader,
        &mut data,
        SocketRecvFlags::default(),
        &never,
    ));
    assert!(poll_once(pending.as_mut()).await.is_none());
    drop(pending);
    let (read, write) = zip(
        scheduler.socket_monitor_recv_poll(
            &mut reader,
            &mut data,
            SocketRecvFlags::default(),
            &never,
        ),
        scheduler.socket_monitor_send_poll(
            &mut sender_write,
            b"z",
            SocketSendFlags::default(),
            &never,
        ),
    )
    .await;
    assert_eq!(read.unwrap(), 1);
    assert_eq!(write.unwrap(), 1);
    assert_eq!(data, [b'z']);
    cancelled(
        scheduler
            .socket_monitor_close(&mut reader, &cancellation)
            .await,
    );
    scheduler
        .socket_monitor_close(&mut reader, &never)
        .await
        .unwrap();
    scheduler
        .socket_monitor_close(&mut writer, &never)
        .await
        .unwrap();
    scheduler
        .socket_monitor_close(&mut sender_read, &never)
        .await
        .unwrap();
    scheduler
        .socket_monitor_close(&mut sender_write, &never)
        .await
        .unwrap();
}

fn cancelled<T>(result: OperationResult<T>) {
    assert!(
        matches!(result, Err(OperationError::Cancelled(_))),
        "expected cooperative cancellation"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_assertion_rejects_progress_and_io_errors() {
        assert!(std::panic::catch_unwind(|| cancelled::<()>(Ok(()))).is_err());
        assert!(
            std::panic::catch_unwind(|| cancelled::<()>(Err(std::io::Error::from(
                std::io::ErrorKind::Interrupted
            )
            .into())))
            .is_err()
        );
    }
}
