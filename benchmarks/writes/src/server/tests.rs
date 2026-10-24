// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use core_scheduler::{
    IoBuf, MonitorStorage, OperationError, OperationResult, SocketAddress, SocketMonitor,
    SocketMonitorState, SocketOperation, SocketRecvMsgPoll, SocketRecvPoll, SocketRecvResult,
    SocketSendMsgPoll, SocketSendPoll, SocketWait, SpawnError, TaskError,
};
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

impl SocketRecvPoll for Script {
    async fn socket_recv_poll(
        &self,
        _: &(),
        buffer: &mut [u8],
        _: SocketRecvFlags,
        _: &Cancellation,
    ) -> OperationResult<usize> {
        let request = self.step("receive")?;
        buffer[0] = request as u8;
        Ok(usize::from(request != 0))
    }
}

impl SocketSendPoll for Script {
    async fn socket_send_poll(
        &self,
        _: &(),
        _: &[u8],
        _: SocketSendFlags,
        _: &Cancellation,
    ) -> OperationResult<usize> {
        Ok(self.step("send")?)
    }
}

impl SocketRecvMsgPoll for Script {
    async fn socket_recvmsg_poll(
        &self,
        _: &(),
        _: &mut [io::IoSliceMut<'_>],
        _: SocketRecvFlags,
        _: &Cancellation,
    ) -> OperationResult<SocketRecvResult> {
        Err(io::Error::from(ErrorKind::Unsupported).into())
    }
}

impl SocketSendMsgPoll for Script {
    async fn socket_sendmsg_poll(
        &self,
        _: &(),
        _: &[io::IoSlice<'_>],
        _: Option<SocketAddress>,
        _: SocketSendFlags,
        _: &Cancellation,
    ) -> OperationResult<usize> {
        Err(io::Error::from(ErrorKind::Unsupported).into())
    }
}

impl SocketWait for Script {
    async fn socket_wait(&self, _: &(), _: Interest, _: &Cancellation) -> OperationResult<()> {
        Err(io::Error::from(ErrorKind::Unsupported).into())
    }
}

impl SocketMonitoring for Script {
    type MonitorRegistration = ();

    fn socket_monitor_state(
        &self,
        _: &(),
        interest: Interest,
    ) -> io::Result<SocketMonitor<(), ()>> {
        self.step("monitor")?;
        Ok(SocketMonitor::new((), interest, ()))
    }

    async fn socket_monitor_close<M>(&self, monitor: &mut MonitorStorage<M>) -> io::Result<()>
    where
        M: SocketMonitorState<(), ()>,
    {
        let interest = monitor.as_ref().as_ref().interest();
        let name = if interest == Interest::Readable {
            "close reader"
        } else {
            "close writer"
        };
        self.step(name).map(|_| ())
    }
}

struct Writer<B>(PhantomData<B>);

impl SocketStreamWriting for Script {
    type Writer<B: IoBuf> = Writer<B>;

    fn socket_stream_output<B: IoBuf>(&self, _: &()) -> io::Result<Writer<B>> {
        self.step("writer")?;
        Ok(Writer(PhantomData))
    }
}

impl<B: IoBuf> SocketStreamOutput<Script> for Writer<B> {
    type Buffer = B;

    async fn submit(
        &mut self,
        scheduler: &Script,
        buffer: B,
        _: &Cancellation,
    ) -> std::result::Result<(), (OperationError, B)> {
        scheduler
            .step("submit")
            .map(|_| ())
            .map_err(|error| (error.into(), buffer))
    }

    async fn flush(&mut self, scheduler: &Script, _: &Cancellation) -> OperationResult<()> {
        scheduler.step("flush").map(|_| ()).map_err(Into::into)
    }

    async fn close(&mut self, scheduler: &Script) -> io::Result<()> {
        scheduler.step("close writer").map(|_| ())
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
        assert_eq!(
            scheduler
                .socket_wait(&(), Interest::Readable, &never)
                .await
                .unwrap_err()
                .kind(),
            ErrorKind::Unsupported
        );
        assert_eq!(
            scheduler
                .socket_recvmsg_poll(&(), &mut [], SocketRecvFlags::default(), &never)
                .await
                .unwrap_err()
                .kind(),
            ErrorKind::Unsupported
        );
        assert_eq!(
            scheduler
                .socket_sendmsg_poll(&(), &[], None, SocketSendFlags::default(), &never)
                .await
                .unwrap_err()
                .kind(),
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
    if core_scheduler_iouring::IoUringSelector::new(Default::default()).is_ok() {
        let error = crate::tests::fail_setup("io_uring runtime", || {
            serve("iouring-writer", 1, WORKLOAD, Vec::new())
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "io_uring runtime");
    }
}
