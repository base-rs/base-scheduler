// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use base_scheduler::*;
use base_scheduler_direct::DirectScheduler;
use futures_lite::future::block_on;
use std::io;

// These must compile through the generic capability contract, where the socket
// is not Copy and its borrow cannot be extended by a concrete implementation.
fn stream<S>(
    scheduler: &S,
    socket: S::Socket,
) -> io::Result<(
    S::Socket,
    impl SocketStreamReader + use<'_, S>,
    impl SocketStreamReader + use<'_, S>,
    impl SocketStreamWriter + use<'_, S>,
    impl SocketStreamWriter<Buffer = Vec<u8>> + use<'_, S>,
)>
where
    S: SocketStreamReading + SocketStreamWriting,
    S::Socket: 'static,
{
    let pool = scheduler.buffer_pool(BufferPoolOptions::new(1, 16))?;
    let reader = scheduler.socket_stream_reader(&socket)?;
    let pooled = scheduler.socket_stream_reader_with_pool(&socket, pool)?;
    let writer = scheduler.socket_stream_writer(&socket)?;
    let typed = scheduler.socket_stream_writer_with_buffer::<Vec<u8>>(&socket)?;
    Ok((socket, reader, pooled, writer, typed))
}

fn packet<S>(
    scheduler: &S,
    socket: S::Socket,
) -> io::Result<(
    S::Socket,
    impl SocketPacketReader + use<'_, S>,
    impl SocketPacketReader + use<'_, S>,
    impl SocketPacketWriter + use<'_, S>,
)>
where
    S: SocketPacketReading + SocketPacketWriting,
    S::Socket: 'static,
{
    let pool = scheduler.buffer_pool(BufferPoolOptions::new(1, 16))?;
    let reader = scheduler.socket_packet_reader(&socket)?;
    let pooled = scheduler.socket_packet_reader_with_pool(&socket, pool)?;
    let writer = scheduler.socket_packet_writer(&socket)?;
    Ok((socket, reader, pooled, writer))
}

#[test]
fn connection_owns_socket_and_all_factory_handle_variants() {
    let scheduler = DirectScheduler::new(Default::default()).unwrap();
    block_on(async {
        let never = Cancellation::never();
        let socket = scheduler
            .socket_open(SocketDomain::Ipv4, SocketType::Stream, None, &never)
            .await
            .unwrap();
        let (socket, mut reader, mut pooled, mut writer, mut typed) =
            stream(&scheduler, socket).unwrap();
        reader.close().await.unwrap();
        pooled.close().await.unwrap();
        writer.close().await.unwrap();
        typed.close().await.unwrap();
        drop((reader, pooled, writer, typed));
        scheduler.socket_close(socket).await.unwrap();
        let socket = scheduler
            .socket_open(SocketDomain::Ipv4, SocketType::Datagram, None, &never)
            .await
            .unwrap();
        let (socket, mut reader, mut pooled, mut writer) = packet(&scheduler, socket).unwrap();
        reader.close().await.unwrap();
        pooled.close().await.unwrap();
        writer.close().await.unwrap();
        drop((reader, pooled, writer));
        scheduler.socket_close(socket).await.unwrap();
        scheduler.shutdown().await;
    });
}
