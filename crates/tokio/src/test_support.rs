// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{TokioScheduler, TokioSocket};
use base_scheduler::{BufferResult, Cancellation, IoBufMut, OperationResult, SocketRecvFlags};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::mpsc,
    time::Duration,
};
pub(crate) fn run(test: impl FnOnce() + Send + 'static) {
    let (done, completion) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(catch_unwind(AssertUnwindSafe(test)));
    });
    if let Err(payload) = completion
        .recv_timeout(Duration::from_secs(15))
        .expect("Tokio adapter test deadline exceeded")
    {
        std::panic::resume_unwind(payload);
    }
}
pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    })
}
pub(crate) fn with_options(options: crate::TokioOptions) -> std::io::Result<crate::TokioScheduler> {
    crate::TokioScheduler::with_options(runtime().handle().clone(), options)
}
pub(crate) fn scheduler() -> crate::TokioScheduler {
    with_options(crate::TokioOptions {
        threads: 1,
        capacity: 4,
    })
    .unwrap()
}

pub(crate) struct Gate {
    entered: mpsc::Sender<()>,
    released: std::sync::Mutex<mpsc::Receiver<()>>,
}
pub(crate) struct Release(mpsc::Sender<()>);
impl Drop for Release {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}
impl Gate {
    pub(crate) fn new() -> (std::sync::Arc<Self>, mpsc::Receiver<()>, Release) {
        let (entered, started) = mpsc::channel();
        let (release, released) = mpsc::channel();
        (
            std::sync::Arc::new(Self {
                entered,
                released: std::sync::Mutex::new(released),
            }),
            started,
            Release(release),
        )
    }
    pub(crate) fn wait(&self) {
        self.entered.send(()).unwrap();
        self.released
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .expect("native operation gate deadline exceeded");
    }
}

#[test]
fn watchdog_propagates_assertion_panics() {
    assert!(catch_unwind(|| run(|| panic!("test assertion"))).is_err());
}

// Stream readers receive through `socket_receive`, selecting their OOB policy.
// These scalar receives exercise it directly.
impl TokioScheduler {
    pub(crate) async fn socket_read<B: IoBufMut>(
        &self,
        socket: &TokioSocket,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.socket_read_with(socket, buffer, cancellation, false)
            .await
    }

    /// `drain_short` is only for OOB-disabled stream readers: a positive short receive clears the readiness it was attempted on.
    pub(crate) async fn socket_read_with<B: IoBufMut>(
        &self,
        socket: &TokioSocket,
        mut buffer: B,
        cancellation: &Cancellation,
        drain_short: bool,
    ) -> BufferResult<B> {
        let length = buffer.as_mut().len();
        let result = self
            .socket_receive(socket, cancellation, length, drain_short, |socket| {
                crate::socket_io::recv(socket, buffer.as_mut())
            })
            .await;
        (result, buffer)
    }

    pub(crate) async fn socket_recv_poll(
        &self,
        socket: &TokioSocket,
        buffer: &mut [u8],
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.socket_recv_poll_with(socket, buffer, flags, cancellation, false)
            .await
    }

    /// `drain_short` is only for OOB-disabled stream readers: a positive short receive clears the readiness it was attempted on.
    pub(crate) async fn socket_recv_poll_with(
        &self,
        socket: &TokioSocket,
        buffer: &mut [u8],
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
        drain_short: bool,
    ) -> OperationResult<usize> {
        let length = buffer.len();
        self.socket_receive(socket, cancellation, length, drain_short, |socket| {
            crate::socket_io::recv_poll(socket, buffer, flags)
        })
        .await
    }
}
