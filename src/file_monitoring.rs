// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::file_monitor::{Activity, MonitoredBuffer, Retirement};
use crate::{
    BufferResult, Cancellation, FileMonitor, FileRead, FileReadAt, FileWrite, FileWriteAt, IoBuf,
    IoBufMut,
};
use std::{
    future::{Future, poll_fn},
    io,
    pin::pin,
    sync::{Arc, atomic::Ordering},
};

/// Optional reusable file operations. A monitor admits one operation at a time, including after abandonment until the backend retires the previous operation. Busy reuse returns `WouldBlock`; it never overwrites kernel-accessible storage.
///
/// The default implementation delegates to ordinary file operations and tracks retirement through their owned buffers. Forgetting a polled fallback future can leak this ownership and permanently prevent reuse or close. Completion backends can reuse their request and completion storage. Shared-cursor ordering and owned-buffer cancellation semantics are unchanged. Construction and unpolled operations submit no work.
pub trait FileMonitoring: FileRead + FileWrite + FileReadAt + FileWriteAt {
    type FileRegistration: Send;

    fn file_monitor(
        &self,
        file: &Self::File,
    ) -> io::Result<FileMonitor<Self::File, Self::FileRegistration>>;

    fn file_monitor_read<B: IoBufMut>(
        &self,
        monitor: &mut FileMonitor<Self::File, Self::FileRegistration>,
        buffer: B,
        cancellation: &Cancellation,
    ) -> impl Future<Output = BufferResult<B>> + Send {
        async move {
            let buffer = match begin(monitor, buffer, cancellation) {
                Ok(buffer) => buffer,
                Err(result) => return result,
            };
            finish(self.file_read(monitor.file(), buffer, cancellation).await)
        }
    }

    fn file_monitor_write<B: IoBuf>(
        &self,
        monitor: &mut FileMonitor<Self::File, Self::FileRegistration>,
        buffer: B,
        cancellation: &Cancellation,
    ) -> impl Future<Output = BufferResult<B>> + Send {
        async move {
            let buffer = match begin(monitor, buffer, cancellation) {
                Ok(buffer) => buffer,
                Err(result) => return result,
            };
            finish(self.file_write(monitor.file(), buffer, cancellation).await)
        }
    }

    fn file_monitor_read_at<B: IoBufMut>(
        &self,
        monitor: &mut FileMonitor<Self::File, Self::FileRegistration>,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> impl Future<Output = BufferResult<B>> + Send {
        async move {
            let buffer = match begin(monitor, buffer, cancellation) {
                Ok(buffer) => buffer,
                Err(result) => return result,
            };
            finish(
                self.file_read_at(monitor.file(), buffer, offset, cancellation)
                    .await,
            )
        }
    }

    fn file_monitor_write_at<B: IoBuf>(
        &self,
        monitor: &mut FileMonitor<Self::File, Self::FileRegistration>,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> impl Future<Output = BufferResult<B>> + Send {
        async move {
            let buffer = match begin(monitor, buffer, cancellation) {
                Ok(buffer) => buffer,
                Err(result) => return result,
            };
            finish(
                self.file_write_at(monitor.file(), buffer, offset, cancellation)
                    .await,
            )
        }
    }

    /// Stop monitor admission and await retirement of its accepted operation. This lazy cleanup takes no cancellation token and remains available after ordinary work is cancelled or scheduler admission closes. Dropping a pending close leaves retirement in progress; await close again to establish completion. Repeated close is safe. Drop the monitor after successful completion to release its retained file before exclusive file close. This does not close the file or affect other monitors.
    fn file_monitor_close(
        &self,
        monitor: &mut FileMonitor<Self::File, Self::FileRegistration>,
    ) -> impl Future<Output = io::Result<()>> + Send {
        async move {
            monitor.closed = true;
            if let Some(activity) = &monitor.activity {
                loop {
                    event_listener::listener!(activity.retired => listener);
                    if !activity.running.load(Ordering::Acquire) {
                        break;
                    }
                    let mut listener = pin!(listener);
                    poll_fn(|context| listener.as_mut().poll(context)).await;
                }
            }
            Ok(())
        }
    }
}

pub(crate) fn begin<F, R, B>(
    monitor: &mut FileMonitor<F, R>,
    buffer: B,
    cancellation: &Cancellation,
) -> Result<MonitoredBuffer<B>, BufferResult<B>> {
    if let Err(error) = cancellation.check() {
        return Err((Err(error.into()), buffer));
    }
    if monitor.closed {
        return Err((
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "file monitor is closed").into()),
            buffer,
        ));
    }
    let activity = monitor
        .activity
        .get_or_insert_with(|| Arc::new(Activity::new()));
    if activity.running.swap(true, Ordering::AcqRel) {
        return Err((
            Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "file monitor operation has not retired",
            )
            .into()),
            buffer,
        ));
    }
    Ok(MonitoredBuffer {
        buffer: Some(buffer),
        _retirement: Retirement(Arc::clone(activity)),
    })
}

pub(crate) fn finish<B>((result, mut buffer): BufferResult<MonitoredBuffer<B>>) -> BufferResult<B> {
    let owner = buffer.buffer.take().unwrap();
    drop(buffer);
    (result, owner)
}
