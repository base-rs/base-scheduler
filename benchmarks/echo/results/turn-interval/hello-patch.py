# Applies the hana-only hello benchmark backends to base-http benchmarks/hello/src/main.rs.
# Usage: python3 hello-patch.py PATH/TO/main.rs
import sys
p=sys.argv[1]
s=open(p).read()
def r(a,b):
    global s
    assert a in s,a; s=s.replace(a,b,1)
# Worker count override for native and io_uring backends.
s=s.replace("NativeExecutorOptions { workers: 1, ..Default::default() }","NativeExecutorOptions { workers: workers(), ..Default::default() }")
r("workers: 1, threads: 1, capacity: MAX_CONNECTIONS,","workers: workers(), threads: 1, capacity: MAX_CONNECTIONS,")
r("""            }, 1)?;""","""            }, workers())?;""")
r("""fn ready(listener: &TcpListener) -> Result<()> {""","""fn workers() -> usize {
    std::env::var("WORKERS").ok().and_then(|value| value.parse().ok()).unwrap_or(1)
}

// Hana experiment: one spawned task per HTTP connection.
#[allow(clippy::manual_async_fn)]
fn base_http_task_server<S>(scheduler: Arc<S>) -> impl std::future::Future<Output = Result<()>> + Send
where
    S: SocketRegistration
        + SocketAccept
        + SocketSetOption<TcpNoDelay>
        + SocketStreamReading
        + SocketStreamWriting
        + SocketClose
        + Spawn
        + Send
        + Sync
        + 'static,
    S::Socket: 'static,
{
    async move {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        ready(&listener)?;
        let listener = scheduler.register_listener(listener)?;
        let cancellation = Cancellation::never();
        let mut connections = FuturesUnordered::new();
        loop {
            if connections.len() == MAX_CONNECTIONS {
                let _ = connections.next().await;
                continue;
            }
            let accepted = {
                let mut accept = pin!(scheduler.socket_accept(&listener, &cancellation));
                loop {
                    let completed = pin!(async {
                        if connections.is_empty() {
                            std::future::pending().await
                        } else {
                            connections.next().await.unwrap()
                        }
                    });
                    match select(accept.as_mut(), completed).await {
                        Either::Left((accepted, _)) => break accepted,
                        Either::Right(_) => {}
                    }
                }
            };
            let (socket, _) = accepted?;
            scheduler
                .socket_set_option(&socket, TcpNoDelay, true, &cancellation)
                .await?;
            let shared = scheduler.clone();
            connections.push(scheduler.spawn(async move {
                let cancellation = Cancellation::never();
                let options = base_http1::Options::default();
                connection_finished(
                    base_http_server::serve_connection(shared.as_ref(), socket, &Hello, &options, &cancellation).await,
                );
            })?);
        }
    }
}

async fn hyper_spawn_server() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    ready(&listener)?;
    listener.set_nonblocking(true)?;
    let listener = tokio::net::TcpListener::from_std(listener)?;
    loop {
        let (socket, _) = listener.accept().await?;
        socket.set_nodelay(true)?;
        tokio::spawn(async move {
            connection_finished(
                http1::Builder::new()
                    .serve_connection(
                        TokioIo::new(socket),
                        service_fn(|_: hyper::Request<hyper::body::Incoming>| async {
                            let mut response = hyper::Response::new(Full::new(Bytes::from_static(BODY)));
                            response.headers_mut().insert(
                                hyper::header::CONTENT_TYPE,
                                hyper::header::HeaderValue::from_static("text/plain"),
                            );
                            Ok::<_, Infallible>(response)
                        }),
                    )
                    .await,
            );
        });
    }
}

fn ready(listener: &TcpListener) -> Result<()> {""")
# native tasks backend
r("""        Some(backend @ ("base-native" | "base-native-background" | "base-native-raw")) => {""","""        Some("base-native-tasks") => {
            let scheduler = Arc::new(NativeScheduler::with_options(NativeOptions {
                workers: workers(), threads: 1, capacity: MAX_CONNECTIONS,
                ..Default::default()
            })?);
            let server = scheduler.spawn(base_http_task_server(scheduler.clone()))?;
            let result = scheduler
                .block_on(server)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            scheduler.block_on(scheduler.shutdown());
            result
        }
        #[cfg(target_os = "linux")]
        Some("base-iouring-tasks") => {
            use base_scheduler_iouring::{IoUringOptions, IoUringSelector};
            let (selector, drivers) = IoUringSelector::with_workers(IoUringOptions {
                capacity: MAX_CONNECTIONS,
                fallback: base_scheduler_selector::SelectorOptions { threads: 1, capacity: MAX_CONNECTIONS },
                ..Default::default()
            }, workers())?;
            let scheduler = Arc::new(NativeScheduler::with_drivers(selector, drivers,
                base_scheduler_native::NativeExecutorOptions { workers: workers(), ..Default::default() })?);
            let server = scheduler.spawn(base_http_task_server(scheduler.clone()))?;
            let result = scheduler.block_on(server)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            scheduler.block_on(scheduler.shutdown());
            result
        }
        Some("hyper-spawn-tokio") => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(workers())
                .enable_all()
                .build()?;
            runtime.block_on(hyper_spawn_server())
        }
        Some(backend @ ("base-native" | "base-native-background" | "base-native-raw")) => {""")
open(p,'w').write(s)
