// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::file_monitoring::{begin, finish};
use crate::{
    BufferResult, Cancellation, FileMonitor, FileMonitoring, FileSubmission, IoBuf, IoBufMut,
};
use std::future::Future;

/// Optional monitor-aware admission/completion hooks. Implementations with custom monitor state must coordinate these operations with ordinary monitor operations and close. The defaults share the portable monitor's owned-buffer retirement guard.
#[doc(hidden)]
pub trait FileMonitorSubmission: FileMonitoring + FileSubmission {
    /// Await bounded admission, then return the accepted operation's owning completion. Monitor reuse remains blocked until the accepted buffer retires, including after dropping its completion. Admission rejection returns the original owner without submitting work.
    fn file_monitor_read_submit<B: IoBufMut>(
        &self,
        monitor: &mut FileMonitor<Self::File, Self::FileRegistration>,
        buffer: B,
        cancellation: &Cancellation,
    ) -> impl Future<
        Output = Result<impl Future<Output = BufferResult<B>> + Send, (crate::OperationError, B)>,
    > + Send {
        async move {
            let buffer = begin(monitor, buffer, cancellation)
                .map_err(|(result, buffer)| (result.unwrap_err(), buffer))?;
            match self
                .file_read_submit(monitor.file(), buffer, cancellation)
                .await
            {
                Ok(completion) => Ok(async move { finish(completion.await) }),
                Err((error, buffer)) => {
                    let (result, buffer) = finish((Err(error), buffer));
                    Err((result.unwrap_err(), buffer))
                }
            }
        }
    }

    /// Await bounded admission, then return the accepted operation's owning completion. Monitor reuse remains blocked until the accepted buffer retires, including after dropping its completion. Admission rejection returns the original owner without submitting work.
    fn file_monitor_write_submit<B: IoBuf>(
        &self,
        monitor: &mut FileMonitor<Self::File, Self::FileRegistration>,
        buffer: B,
        cancellation: &Cancellation,
    ) -> impl Future<
        Output = Result<impl Future<Output = BufferResult<B>> + Send, (crate::OperationError, B)>,
    > + Send {
        async move {
            let buffer = begin(monitor, buffer, cancellation)
                .map_err(|(result, buffer)| (result.unwrap_err(), buffer))?;
            match self
                .file_write_submit(monitor.file(), buffer, cancellation)
                .await
            {
                Ok(completion) => Ok(async move { finish(completion.await) }),
                Err((error, buffer)) => {
                    let (result, buffer) = finish((Err(error), buffer));
                    Err((result.unwrap_err(), buffer))
                }
            }
        }
    }

    /// Await bounded admission, then return the accepted operation's owning completion. Monitor reuse remains blocked until the accepted buffer retires, including after dropping its completion. Admission rejection returns the original owner without submitting work.
    fn file_monitor_read_at_submit<B: IoBufMut>(
        &self,
        monitor: &mut FileMonitor<Self::File, Self::FileRegistration>,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> impl Future<
        Output = Result<impl Future<Output = BufferResult<B>> + Send, (crate::OperationError, B)>,
    > + Send {
        async move {
            let buffer = begin(monitor, buffer, cancellation)
                .map_err(|(result, buffer)| (result.unwrap_err(), buffer))?;
            match self
                .file_read_at_submit(monitor.file(), buffer, offset, cancellation)
                .await
            {
                Ok(completion) => Ok(async move { finish(completion.await) }),
                Err((error, buffer)) => {
                    let (result, buffer) = finish((Err(error), buffer));
                    Err((result.unwrap_err(), buffer))
                }
            }
        }
    }

    /// Await bounded admission, then return the accepted operation's owning completion. Monitor reuse remains blocked until the accepted buffer retires, including after dropping its completion. Admission rejection returns the original owner without submitting work.
    fn file_monitor_write_at_submit<B: IoBuf>(
        &self,
        monitor: &mut FileMonitor<Self::File, Self::FileRegistration>,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> impl Future<
        Output = Result<impl Future<Output = BufferResult<B>> + Send, (crate::OperationError, B)>,
    > + Send {
        async move {
            let buffer = begin(monitor, buffer, cancellation)
                .map_err(|(result, buffer)| (result.unwrap_err(), buffer))?;
            match self
                .file_write_at_submit(monitor.file(), buffer, offset, cancellation)
                .await
            {
                Ok(completion) => Ok(async move { finish(completion.await) }),
                Err((error, buffer)) => {
                    let (result, buffer) = finish((Err(error), buffer));
                    Err((result.unwrap_err(), buffer))
                }
            }
        }
    }
}
