// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::Result;
use core_scheduler::{
    Cancellation, IoBuf, SocketAccept, SocketRead, SocketRecvFlags, SocketRecvPoll,
    SocketRegistration, SocketSendFlags, SocketSendPoll, SocketSetOption, SocketWrite, Spawn,
    TcpNoDelay,
};
use core_scheduler_tokio::TokioScheduler;
use std::{
    io::{self, Write},
    net::TcpListener,
    sync::Arc,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const BUFFER_SIZE: usize = 65536;

fn ready(listener: &TcpListener) {
    println!("{}", listener.local_addr().expect("listener address"));
    io::stdout().flush().expect("report listener address");
}

async fn tokio_server(listener: TcpListener, connections: usize) {
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let listener =
        tokio::net::TcpListener::from_std(listener).expect("Tokio listener registration");
    let mut tasks = Vec::with_capacity(connections);
    for _ in 0..connections {
        let (mut socket, _) = listener.accept().await.expect("accept connection");
        socket.set_nodelay(true).expect("TCP_NODELAY");
        tasks.push(tokio::spawn(async move {
            let mut buffer = vec![0; BUFFER_SIZE];
            loop {
                let count = socket.read(&mut buffer).await.expect("echo read");
                if count == 0 {
                    return;
                }
                socket
                    .write_all(&buffer[..count])
                    .await
                    .expect("echo write");
            }
        }));
    }
    for task in tasks {
        task.await.expect("echo task failed");
    }
}

async fn echo<S: SocketRead + SocketWrite>(scheduler: &S, socket: S::Socket) {
    let never = Cancellation::never();
    let mut buffer = vec![0; BUFFER_SIZE];
    loop {
        let (result, returned) = scheduler.socket_read(&socket, buffer, &never).await;
        buffer = returned;
        let count = result.expect("echo operation");
        if count == 0 {
            return;
        }
        let mut sent = 0;
        while sent < count {
            let (result, returned) = scheduler
                .socket_write(&socket, buffer.slice(sent..count), &never)
                .await;
            buffer = returned.into_inner();
            let count = result.expect("echo operation");
            assert!(count != 0, "echo write made zero progress");
            sent += count;
        }
    }
}

async fn echo_poll<S: SocketRecvPoll + SocketSendPoll>(scheduler: &S, socket: S::Socket) {
    let never = Cancellation::never();
    let mut buffer = vec![0; BUFFER_SIZE];
    loop {
        let count = scheduler
            .socket_recv_poll(&socket, &mut buffer, SocketRecvFlags::default(), &never)
            .await
            .expect("echo operation");
        if count == 0 {
            return;
        }
        let mut sent = 0;
        while sent < count {
            let count = scheduler
                .socket_send_poll(
                    &socket,
                    &buffer[sent..count],
                    SocketSendFlags::default(),
                    &never,
                )
                .await
                .expect("echo operation");
            assert!(count != 0, "echo write made zero progress");
            sent += count;
        }
    }
}

async fn scheduler_server<S>(
    scheduler: Arc<S>,
    listener: TcpListener,
    connections: usize,
    poll: bool,
) where
    S: SocketRegistration
        + SocketAccept
        + SocketRead
        + SocketWrite
        + SocketRecvPoll
        + SocketSendPoll
        + SocketSetOption<TcpNoDelay>
        + Spawn
        + Send
        + Sync
        + 'static,
{
    let listener = scheduler
        .register_listener(listener)
        .expect("scheduler listener registration");
    let mut tasks = Vec::with_capacity(connections);
    let never = Cancellation::never();
    for _ in 0..connections {
        let (socket, _) = scheduler
            .socket_accept(&listener, &never)
            .await
            .expect("accept connection");
        scheduler
            .socket_set_option(&socket, TcpNoDelay, true, &never)
            .await
            .expect("TCP_NODELAY");
        let shared = Arc::clone(&scheduler);
        let task = if poll {
            scheduler.spawn(async move { echo_poll(shared.as_ref(), socket).await })
        } else {
            scheduler.spawn(async move { echo(shared.as_ref(), socket).await })
        };
        tasks.push(task.expect("spawn echo task"));
    }
    for task in tasks {
        task.await.expect("echo task failed");
    }
}

pub(crate) fn run(backend: &str, workers: usize, connections: usize) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind echo listener");
    match backend {
        "tokio" => {
            let runtime = crate::runtime(workers);
            ready(&listener);
            runtime.block_on(tokio_server(listener, connections));
        }
        "tokio-adapter" | "tokio-adapter-poll" => {
            let runtime = crate::runtime(workers);
            let scheduler = Arc::new(TokioScheduler::new(runtime.handle().clone()));
            ready(&listener);
            runtime.block_on(scheduler_server(
                Arc::clone(&scheduler),
                listener,
                connections,
                backend == "tokio-adapter-poll",
            ));
            runtime.block_on(scheduler.shutdown());
        }
        #[cfg(target_os = "linux")]
        "iouring" | "iouring-poll" => {
            use core_scheduler_iouring::{IoUringOptions, IoUringSelector};
            use core_scheduler_native::{NativeExecutorOptions, NativeScheduler};
            let (selector, drivers) = IoUringSelector::with_workers(
                IoUringOptions {
                    capacity: connections.max(256),
                    ..Default::default()
                },
                workers,
            )
            .expect("io_uring setup");
            let scheduler = Arc::new(
                NativeScheduler::with_drivers(
                    selector,
                    drivers,
                    NativeExecutorOptions {
                        workers,
                        ..Default::default()
                    },
                )
                .expect("Native runtime setup"),
            );
            ready(&listener);
            scheduler.block_on(scheduler_server(
                Arc::clone(&scheduler),
                listener,
                connections,
                backend == "iouring-poll",
            ));
            scheduler.block_on(scheduler.shutdown());
        }
        _ => return Err("unknown backend (iouring requires Linux)".into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests;
