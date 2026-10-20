from pathlib import Path
root=Path('/home/samuel/Developer/core-rs/core-scheduler-perf-native-next')
p=root/'crates/iouring/src/io_uring_selector.rs'
s=p.read_text().replace('async fn submit<T>(\n','async fn submit<T, E: std::fmt::Debug>(\n').replace('result: oneshot::Receiver<T>,','result: impl Future<Output = Result<T, E>>,')
p.write_text(s)
p=root/'crates/iouring/src/io_uring_selector/readiness.rs'
s=p.read_text().replace('use std::io::{IoSlice, IoSliceMut};','''use std::io::{IoSlice, IoSliceMut};
use std::sync::Mutex;
use std::task::{Context, Waker};
use std::pin::Pin;

enum CompletionState {
    Pending(Option<Waker>),
    Ready(io::Result<usize>),
    Disconnected,
}

struct ReadinessSender(Option<Arc<Mutex<CompletionState>>>);
struct ReadinessReceiver(Arc<Mutex<CompletionState>>);

fn completion() -> (ReadinessSender, ReadinessReceiver) {
    let state = Arc::new(Mutex::new(CompletionState::Pending(None)));
    (ReadinessSender(Some(Arc::clone(&state))), ReadinessReceiver(state))
}

impl ReadinessSender {
    fn send(mut self, result: io::Result<usize>) {
        let state = self.0.take().expect("sender completes once");
        let previous = std::mem::replace(&mut *state.lock().unwrap(), CompletionState::Ready(result));
        if let CompletionState::Pending(Some(waker)) = previous {
            waker.wake();
        }
    }
}

impl Drop for ReadinessSender {
    fn drop(&mut self) {
        if let Some(state) = self.0.take() {
            let previous = std::mem::replace(&mut *state.lock().unwrap(), CompletionState::Disconnected);
            if let CompletionState::Pending(Some(waker)) = previous {
                waker.wake();
            }
        }
    }
}

impl Future for ReadinessReceiver {
    type Output = Result<io::Result<usize>, &'static str>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.0.lock().unwrap();
        match &mut *state {
            CompletionState::Pending(waker) => {
                match waker {
                    Some(waker) => waker.clone_from(context.waker()),
                    None => *waker = Some(context.waker().clone()),
                }
                Poll::Pending
            }
            CompletionState::Ready(_) => {
                let CompletionState::Ready(result) = std::mem::replace(&mut *state, CompletionState::Disconnected) else { unreachable!() };
                Poll::Ready(Ok(result))
            }
            CompletionState::Disconnected => Poll::Ready(Err("readiness sender disconnected")),
        }
    }
}

impl Drop for ReadinessReceiver {
    fn drop(&mut self) {
        if let CompletionState::Pending(waker) = &mut *self.0.lock().unwrap() {
            *waker = None;
        }
    }
}''').replace('reply: Cell<Option<oneshot::Sender<io::Result<usize>>>>,','reply: Cell<Option<ReadinessSender>>,').replace('discard(reply.send(result));','reply.send(result);').replace('discard(reply.send(Err(io::Error::new(','reply.send(Err(io::Error::new(').replace('            ))));','            )));').replace('let (reply, result) = oneshot::channel();','let (reply, result) = completion();')
p.write_text(s)
