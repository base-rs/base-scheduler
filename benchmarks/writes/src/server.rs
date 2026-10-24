// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{Result, Workload, prepare};
use core_scheduler::{
    BufferLease, Cancellation, HeapBufferPool, Interest, SocketMonitoring, SocketRecvFlags,
    SocketRegistration, SocketSendFlags, SocketStreamWriter, SocketStreamWriting, Spawn,
};
use core_scheduler_tokio::TokioScheduler;
use std::{
    future::Future,
    io::{self, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn tokio_connection(socket: TcpStream, workload: Workload) -> Result<()> {
    socket.set_nonblocking(true)?;
    let mut socket = tokio::net::TcpStream::from_std(socket)?;
    let pool = HeapBufferPool::new(2, workload.bytes)?;
    let never = Cancellation::never();
    let mut request = [0];
    while socket.read(&mut request).await? != 0 {
        if !matches!(request[0], 1 | 2) {
            return Err("invalid request".into());
        }
        for chunk in 0..workload.chunks {
            let mut buffer = pool.acquire(&never).await?;
            prepare(
                buffer.as_mut(),
                chunk + usize::from(request[0]) * workload.chunks,
                workload.work,
            );
            socket.write_all(buffer.as_ref()).await?;
        }
    }
    Ok(())
}

async fn monitor_connection<S>(
    scheduler: Arc<S>,
    socket: S::Socket,
    workload: Workload,
) -> Result<()>
where
    S: SocketMonitoring + 'static,
    S::Socket: 'static,
{
    let pool = HeapBufferPool::new(2, workload.bytes)?;
    let never = Cancellation::never();
    let mut reader = scheduler.socket_stream_monitor(&socket, Interest::Readable)?;
    let mut writer = scheduler.socket_stream_monitor(&socket, Interest::Writable)?;
    let result = async {
        let mut request = [0];
        while scheduler
            .socket_monitor_recv_poll(
                &mut reader,
                &mut request,
                SocketRecvFlags::default(),
                &never,
            )
            .await?
            != 0
        {
            if !matches!(request[0], 1 | 2) {
                return Err("invalid request".into());
            }
            for chunk in 0..workload.chunks {
                let mut buffer = pool.acquire(&never).await?;
                prepare(
                    buffer.as_mut(),
                    chunk + usize::from(request[0]) * workload.chunks,
                    workload.work,
                );
                let mut sent = 0;
                while sent < workload.bytes {
                    let count = scheduler
                        .socket_monitor_send_poll(
                            &mut writer,
                            &buffer.as_ref()[sent..],
                            SocketSendFlags::default(),
                            &never,
                        )
                        .await?;
                    if count == 0 {
                        return Err(io::Error::from(io::ErrorKind::WriteZero).into());
                    }
                    sent += count;
                }
            }
        }
        Ok(())
    }
    .await;
    let reader_closed = scheduler.socket_monitor_close(&mut reader).await;
    let writer_closed = scheduler.socket_monitor_close(&mut writer).await;
    reader_closed?;
    writer_closed?;
    result
}

async fn writer_connection<S>(
    scheduler: Arc<S>,
    socket: S::Socket,
    workload: Workload,
) -> Result<()>
where
    S: SocketMonitoring + SocketStreamWriting + 'static,
    S::Socket: 'static,
{
    let pool = HeapBufferPool::new(2, workload.bytes)?;
    let never = Cancellation::never();
    let mut reader = scheduler.socket_stream_monitor(&socket, Interest::Readable)?;
    let mut writer = scheduler.socket_stream_writer::<BufferLease>(&socket)?;
    let result = async {
        let mut request = [0];
        while scheduler
            .socket_monitor_recv_poll(
                &mut reader,
                &mut request,
                SocketRecvFlags::default(),
                &never,
            )
            .await?
            != 0
        {
            if !matches!(request[0], 1 | 2) {
                return Err("invalid request".into());
            }
            for chunk in 0..workload.chunks {
                let mut buffer = pool.acquire(&never).await?;
                prepare(
                    buffer.as_mut(),
                    chunk + usize::from(request[0]) * workload.chunks,
                    workload.work,
                );
                writer
                    .submit(scheduler.as_ref(), buffer, &never)
                    .await
                    .map_err(|(error, _buffer)| error)?;
            }
            writer.flush(scheduler.as_ref(), &never).await?;
        }
        Ok(())
    }
    .await;
    let reader_closed = scheduler.socket_monitor_close(&mut reader).await;
    let writer_closed = writer.close(scheduler.as_ref()).await;
    reader_closed?;
    writer_closed?;
    result
}

async fn scheduler_server<S, F, C>(
    scheduler: Arc<S>,
    sockets: Vec<TcpStream>,
    workload: Workload,
    connection: C,
) -> Result<()>
where
    S: SocketRegistration + Spawn + 'static,
    S::Socket: 'static,
    C: Fn(Arc<S>, S::Socket, Workload) -> F,
    F: Future<Output = Result<()>> + Send + 'static,
{
    let mut tasks = Vec::with_capacity(sockets.len());
    for socket in sockets {
        let socket = scheduler.register_socket(socket)?;
        tasks.push(scheduler.spawn(connection(Arc::clone(&scheduler), socket, workload))?);
    }
    for task in tasks {
        task.await
            .map_err(|error| io::Error::other(error.to_string()))??;
    }
    Ok(())
}

pub(crate) fn run(
    backend: &str,
    workers: usize,
    connections: usize,
    workload: Workload,
    send_buffer: usize,
) -> Result<()> {
    if !matches!(
        backend,
        "tokio"
            | "tokio-monitor"
            | "tokio-writer"
            | "native-monitor"
            | "iouring-monitor"
            | "iouring-writer"
    ) {
        return Err("unknown write benchmark backend".into());
    }
    let listener = TcpListener::bind("127.0.0.1:0")?;
    println!("{}", listener.local_addr()?);
    io::stdout().flush()?;
    // Accept and configure all streams before timing starts. Each accepted stream is registered only once, after its mode and options have been set.
    let mut sockets = Vec::with_capacity(connections);
    for _ in 0..connections {
        let (socket, _) = listener.accept()?;
        socket.set_nodelay(true)?;
        let native = socket2::SockRef::from(&socket);
        if send_buffer != 0 {
            native.set_send_buffer_size(send_buffer)?;
        }
        if sockets.is_empty() {
            eprintln!(
                "{}",
                serde_json::json!({"requested_send_buffer": send_buffer, "actual_send_buffer": native.send_buffer_size()?})
            );
        }
        sockets.push(socket);
    }
    match backend {
        "tokio" => crate::runtime(workers).block_on(async move {
            let tasks: Vec<_> = sockets
                .into_iter()
                .map(|socket| tokio::spawn(tokio_connection(socket, workload)))
                .collect();
            for task in tasks {
                task.await??;
            }
            Ok(())
        }),
        "tokio-monitor" | "tokio-writer" => {
            let runtime = crate::runtime(workers);
            let scheduler = Arc::new(TokioScheduler::new(runtime.handle().clone()));
            let result = if backend == "tokio-writer" {
                runtime.block_on(scheduler_server(
                    Arc::clone(&scheduler),
                    sockets,
                    workload,
                    writer_connection,
                ))
            } else {
                runtime.block_on(scheduler_server(
                    Arc::clone(&scheduler),
                    sockets,
                    workload,
                    monitor_connection,
                ))
            };
            runtime.block_on(scheduler.shutdown());
            result
        }
        #[cfg(target_os = "linux")]
        "native-monitor" => {
            use core_scheduler_native::{NativeOptions, NativeScheduler};
            let scheduler = Arc::new(NativeScheduler::with_options(NativeOptions {
                workers,
                ..Default::default()
            })?);
            let result = scheduler.block_on(scheduler_server(
                Arc::clone(&scheduler),
                sockets,
                workload,
                monitor_connection,
            ));
            scheduler.block_on(scheduler.shutdown());
            result
        }
        #[cfg(target_os = "linux")]
        "iouring-monitor" | "iouring-writer" => {
            use core_scheduler_iouring::{IoUringOptions, IoUringSelector};
            use core_scheduler_native::{NativeExecutorOptions, NativeScheduler};
            let (selector, drivers) = IoUringSelector::with_workers(
                IoUringOptions {
                    capacity: (connections * 2).max(256),
                    ..Default::default()
                },
                workers,
            )?;
            let scheduler = Arc::new(NativeScheduler::with_drivers(
                selector,
                drivers,
                NativeExecutorOptions {
                    workers,
                    ..Default::default()
                },
            )?);
            let result = if backend == "iouring-writer" {
                scheduler.block_on(scheduler_server(
                    Arc::clone(&scheduler),
                    sockets,
                    workload,
                    writer_connection,
                ))
            } else {
                scheduler.block_on(scheduler_server(
                    Arc::clone(&scheduler),
                    sockets,
                    workload,
                    monitor_connection,
                ))
            };
            scheduler.block_on(scheduler.shutdown());
            result
        }
        _ => Err("Native backends require Linux".into()),
    }
}
