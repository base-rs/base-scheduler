// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use base_scheduler::*;
use std::{collections::VecDeque, io::ErrorKind, marker::PhantomData, pin::Pin, sync::Mutex};

// A script describes one observable operation per step. It verifies retirement order as well as failures, without replacing any production syscall.
struct Script(Mutex<VecDeque<(&'static str, io::Result<usize>)>>);

impl Script {
    fn new(steps: impl IntoIterator<Item = (&'static str, io::Result<usize>)>) -> Arc<Self> {
        Arc::new(Self(Mutex::new(steps.into_iter().collect())))
    }

    fn step(&self, operation: &str) -> io::Result<usize> {
        let (expected, result) = self.0.lock().unwrap().pop_front().unwrap();
        assert_eq!(operation, expected);
        result
    }

    fn finished(&self) {
        assert!(self.0.lock().unwrap().is_empty());
    }
}

impl SocketOperation for Script {
    type Socket = ();
}

impl BufferPooling for Script {
    type Pool = HeapBufferPool;
    fn buffer_pool(&self, options: BufferPoolOptions) -> io::Result<Self::Pool> {
        HeapBufferPool::with_options(options)
    }
}
struct Reader<'a>(&'a Script);
impl SocketStreamReader for Reader<'_> {
    type Buffer = BufferLease;
    async fn read(&mut self, _: &Cancellation) -> OperationResult<Option<BufferLease>> {
        Err(io::Error::from(ErrorKind::Unsupported).into())
    }
    async fn read_into(&mut self, buffer: &mut [u8], _: &Cancellation) -> OperationResult<usize> {
        let request = self.0.step("receive")?;
        buffer[0] = request as u8;
        Ok(usize::from(request != 0))
    }
    async fn read_vectored(
        &mut self,
        _: &mut [io::IoSliceMut<'_>],
        _: &Cancellation,
    ) -> OperationResult<usize> {
        Err(io::Error::from(ErrorKind::Unsupported).into())
    }
    async fn read_vectored_with_flags(
        &mut self,
        _: &mut [io::IoSliceMut<'_>],
        _: SocketRecvFlags,
        _: &Cancellation,
    ) -> OperationResult<SocketRecvResult> {
        Err(io::Error::from(ErrorKind::Unsupported).into())
    }
    async fn read_owned_vectored<B: IoBufSequenceMut>(
        &mut self,
        buffers: B,
        _: SocketRecvFlags,
        _: &Cancellation,
    ) -> (OperationResult<SocketRecvResult>, B) {
        (Err(io::Error::from(ErrorKind::Unsupported).into()), buffers)
    }
    async fn close(&mut self) -> io::Result<()> {
        self.0.step("close reader").map(|_| ())
    }
}
struct Writer<'a, B: IoBuf>(&'a Script, PhantomData<B>);
impl<B: IoBuf> SocketStreamWriter for Writer<'_, B> {
    type Buffer = B;
    async fn write(&mut self, _: &[u8], _: &Cancellation) -> OperationResult<usize> {
        Ok(self.0.step("send")?)
    }
    async fn write_vectored(
        &mut self,
        _: &[io::IoSlice<'_>],
        _: &Cancellation,
    ) -> OperationResult<usize> {
        Err(io::Error::from(ErrorKind::Unsupported).into())
    }
    async fn submit(
        &mut self,
        buffer: B,
        _: &Cancellation,
    ) -> std::result::Result<(), (OperationError, B)> {
        self.0
            .step("submit")
            .map(|_| ())
            .map_err(|error| (error.into(), buffer))
    }
    async fn flush(&mut self, _: &Cancellation) -> OperationResult<()> {
        self.0.step("flush").map(|_| ()).map_err(Into::into)
    }
    async fn write_vectored_with_flags(
        &mut self,
        _: &[io::IoSlice<'_>],
        _: SocketSendFlags,
        _: &Cancellation,
    ) -> OperationResult<usize> {
        Err(io::Error::from(ErrorKind::Unsupported).into())
    }
    async fn write_owned_vectored<Buffers: IoBufSequence>(
        &mut self,
        buffers: Buffers,
        _: SocketSendFlags,
        _: &Cancellation,
    ) -> (OperationResult<usize>, Buffers) {
        (Err(io::Error::from(ErrorKind::Unsupported).into()), buffers)
    }
    async fn close(&mut self) -> io::Result<()> {
        self.0.step("close writer").map(|_| ())
    }
}
impl SocketStreamReading for Script {
    fn socket_stream_reader<'a>(
        &'a self,
        _: &(),
    ) -> io::Result<impl SocketStreamReader<Buffer = BufferLease> + 'a + use<'a>> {
        self.step("monitor")?;
        Ok(Reader(self))
    }
    fn socket_stream_reader_with_pool<'a>(
        &'a self,
        _: &(),
        _: Self::Pool,
    ) -> io::Result<impl SocketStreamReader<Buffer = BufferLease> + 'a + use<'a>> {
        self.step("monitor")?;
        Ok(Reader(self))
    }
}
impl SocketStreamWriting for Script {
    fn socket_stream_writer<'a>(
        &'a self,
        _: &(),
    ) -> io::Result<impl SocketStreamWriter<Buffer = BufferLease> + 'a + use<'a>> {
        self.step("monitor")?;
        Ok(Writer(self, PhantomData))
    }
    fn socket_stream_writer_with_buffer<'a, B: IoBuf>(
        &'a self,
        _: &(),
    ) -> io::Result<impl SocketStreamWriter<Buffer = B> + 'a + use<'a, B>> {
        self.step("writer")?;
        Ok(Writer(self, PhantomData))
    }
}

impl SocketRegistration for Script {
    fn register_socket(&self, _: TcpStream) -> io::Result<()> {
        self.step("register").map(|_| ())
    }

    fn register_listener(&self, _: TcpListener) -> io::Result<()> {
        Err(io::Error::from(ErrorKind::Unsupported))
    }
}

impl Spawn for Script {
    type Handle<T: Send + 'static> =
        Pin<Box<dyn Future<Output = std::result::Result<T, TaskError>> + Send>>;

    fn spawn<F>(&self, future: F) -> std::result::Result<Self::Handle<F::Output>, SpawnError>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let completion = self
            .step("spawn")
            .map_err(|_| SpawnError::SchedulerClosed)?;
        Ok(Box::pin(async move {
            if completion == 0 {
                Err(TaskError::Cancelled)
            } else {
                Ok(future.await)
            }
        }))
    }
}

fn failure(message: &'static str) -> io::Result<usize> {
    Err(io::Error::other(message))
}

const WORKLOAD: Workload = Workload {
    bytes: 3,
    chunks: 1,
    work: 0,
};

#[test]
fn monitor_failures_still_close_both_directions_and_prioritize_cleanup_errors() {
    let runtime = crate::runtime(1);
    for (steps, expected) in [
        (vec![("monitor", failure("reader setup"))], "reader setup"),
        (
            vec![("monitor", Ok(0)), ("monitor", failure("writer setup"))],
            "writer setup",
        ),
        (
            vec![
                ("monitor", Ok(0)),
                ("monitor", Ok(0)),
                ("receive", failure("receive")),
                ("close reader", Ok(0)),
                ("close writer", Ok(0)),
            ],
            "receive",
        ),
        (
            vec![
                ("monitor", Ok(0)),
                ("monitor", Ok(0)),
                ("receive", Ok(1)),
                ("send", failure("send")),
                ("close reader", Ok(0)),
                ("close writer", Ok(0)),
            ],
            "send",
        ),
        (
            vec![
                ("monitor", Ok(0)),
                ("monitor", Ok(0)),
                ("receive", Ok(1)),
                ("send", Ok(0)),
                ("close reader", Ok(0)),
                ("close writer", Ok(0)),
            ],
            "write zero",
        ),
        (
            vec![
                ("monitor", Ok(0)),
                ("monitor", Ok(0)),
                ("receive", failure("receive")),
                ("close reader", failure("reader retirement")),
                ("close writer", failure("writer retirement")),
            ],
            "reader retirement",
        ),
        (
            vec![
                ("monitor", Ok(0)),
                ("monitor", Ok(0)),
                ("receive", Ok(0)),
                ("close reader", Ok(0)),
                ("close writer", failure("writer retirement")),
            ],
            "writer retirement",
        ),
    ] {
        let scheduler = Script::new(steps);
        let error = runtime
            .block_on(monitor_connection::<_, false>(
                Arc::clone(&scheduler),
                (),
                WORKLOAD,
            ))
            .unwrap_err();
        if expected == "write zero" {
            assert_eq!(
                error.downcast::<io::Error>().unwrap().kind(),
                ErrorKind::WriteZero
            );
        } else {
            assert_eq!(error.to_string(), expected);
        }
        scheduler.finished();
    }
}

#[test]
fn monitor_retries_partial_writes_before_reading_the_next_request() {
    let scheduler = Script::new([
        ("monitor", Ok(0)),
        ("monitor", Ok(0)),
        ("receive", Ok(1)),
        ("send", Ok(1)),
        ("send", Ok(2)),
        ("receive", Ok(0)),
        ("close reader", Ok(0)),
        ("close writer", Ok(0)),
    ]);
    crate::runtime(1)
        .block_on(monitor_connection::<_, true>(
            Arc::clone(&scheduler),
            (),
            WORKLOAD,
        ))
        .unwrap();
    scheduler.finished();
}

#[test]
fn writer_failures_still_retire_both_directions() {
    let runtime = crate::runtime(1);
    for (steps, expected) in [
        (vec![("monitor", failure("reader setup"))], "reader setup"),
        (
            vec![("monitor", Ok(0)), ("writer", failure("writer setup"))],
            "writer setup",
        ),
        (
            vec![
                ("monitor", Ok(0)),
                ("writer", Ok(0)),
                ("receive", failure("receive")),
                ("close reader", Ok(0)),
                ("close writer", Ok(0)),
            ],
            "receive",
        ),
        (
            vec![
                ("monitor", Ok(0)),
                ("writer", Ok(0)),
                ("receive", Ok(1)),
                ("submit", failure("submission")),
                ("close reader", Ok(0)),
                ("close writer", Ok(0)),
            ],
            "submission",
        ),
        (
            vec![
                ("monitor", Ok(0)),
                ("writer", Ok(0)),
                ("receive", Ok(1)),
                ("submit", Ok(0)),
                ("flush", failure("flush")),
                ("close reader", Ok(0)),
                ("close writer", Ok(0)),
            ],
            "flush",
        ),
        (
            vec![
                ("monitor", Ok(0)),
                ("writer", Ok(0)),
                ("receive", failure("receive")),
                ("close reader", failure("reader retirement")),
                ("close writer", failure("writer retirement")),
            ],
            "reader retirement",
        ),
        (
            vec![
                ("monitor", Ok(0)),
                ("writer", Ok(0)),
                ("receive", Ok(0)),
                ("close reader", Ok(0)),
                ("close writer", failure("writer retirement")),
            ],
            "writer retirement",
        ),
    ] {
        let scheduler = Script::new(steps);
        let error = runtime
            .block_on(writer_connection(Arc::clone(&scheduler), (), WORKLOAD))
            .unwrap_err();
        assert_eq!(error.to_string(), expected);
        scheduler.finished();
    }
}

#[test]
fn scheduler_admission_and_task_failures_are_returned() {
    let runtime = crate::runtime(1);
    for (steps, expected) in [
        (vec![("register", failure("registration"))], "registration"),
        (
            vec![("register", Ok(0)), ("spawn", failure("closed"))],
            "the scheduler is closed",
        ),
        (
            vec![("register", Ok(0)), ("spawn", Ok(0))],
            "task cancelled",
        ),
        (
            vec![("register", Ok(0)), ("spawn", Ok(1))],
            "connection failed",
        ),
    ] {
        let scheduler = Script::new(steps);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let _accepted = listener.accept().unwrap();
        let error = runtime
            .block_on(scheduler_server(
                Arc::clone(&scheduler),
                vec![socket],
                WORKLOAD,
                |_, _, _| async { Err("connection failed".into()) },
            ))
            .unwrap_err();
        assert_eq!(error.to_string(), expected);
        scheduler.finished();
    }
}

#[test]
fn script_rejects_operations_outside_the_benchmark_contract() {
    let scheduler = Script::new([]);
    let runtime = crate::runtime(1);
    runtime.block_on(async {
        let never = Cancellation::never();
        let mut reader = Reader(scheduler.as_ref());
        assert_eq!(
            reader
                .read_vectored(&mut [], &never)
                .await
                .unwrap_err()
                .kind(),
            ErrorKind::Unsupported
        );
        let mut writer = Writer::<Vec<u8>>(scheduler.as_ref(), PhantomData);
        assert_eq!(
            writer.write_vectored(&[], &never).await.unwrap_err().kind(),
            ErrorKind::Unsupported
        );
    });
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    assert_eq!(
        scheduler.register_listener(listener).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
    scheduler.finished();
}

#[test]
fn setup_failures_are_reported_before_connections_start() {
    for operation in [
        "bind listener",
        "listener address",
        "report listener",
        "accept connection",
    ] {
        let error =
            crate::tests::fail_setup(operation, || run("tokio", 1, 1, WORKLOAD, 0)).unwrap_err();
        assert_eq!(error.to_string(), operation);
    }
    for operation in [
        "accept connection",
        "server TCP_NODELAY",
        "send buffer",
        "send buffer size",
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let error = crate::tests::fail_setup(operation, || accept_connections(&listener, 1, 65536))
            .unwrap_err();
        assert_eq!(error.to_string(), operation);
    }
    let runtime = crate::runtime(1);
    for operation in ["nonblocking stream", "Tokio stream registration"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (_peer, _) = listener.accept().unwrap();
        let error = crate::tests::fail_setup(operation, || {
            runtime.block_on(tokio_connection(client, WORKLOAD))
        })
        .unwrap_err();
        assert_eq!(error.to_string(), operation);
    }
}

#[test]
fn tokio_connection_reports_a_reset_while_waiting_for_a_request() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (socket, _) = listener.accept().unwrap();
    crate::runtime(1).block_on(async {
        let mut connection = std::pin::pin!(tokio_connection(socket, WORKLOAD));
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(connection.as_mut().poll(&mut context).is_pending());
        socket2::SockRef::from(&peer)
            .set_linger(Some(std::time::Duration::ZERO))
            .unwrap();
        drop(peer);
        let error = connection.await.unwrap_err();
        assert_eq!(
            error.downcast::<io::Error>().unwrap().kind(),
            ErrorKind::ConnectionReset
        );
    });
}

#[cfg(target_os = "linux")]
#[test]
fn native_runtime_setup_failures_are_returned() {
    for (backend, operation) in [
        ("native-monitor", "native runtime"),
        ("iouring-writer", "io_uring selector"),
    ] {
        let error = crate::tests::fail_setup(operation, || serve(backend, 1, WORKLOAD, Vec::new()))
            .unwrap_err();
        assert_eq!(error.to_string(), operation);
    }
    if base_scheduler_iouring::IoUringSelector::new(Default::default()).is_ok() {
        let error = crate::tests::fail_setup("io_uring runtime", || {
            serve("iouring-writer", 1, WORKLOAD, Vec::new())
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "io_uring runtime");
    }
}
