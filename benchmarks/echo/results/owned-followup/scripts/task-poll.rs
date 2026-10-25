// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Compare combined task/executor overhead without I/O, timers, or per-poll shared counters.

use std::{
    fmt::Debug,
    future::{Future, poll_fn},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Waker},
    time::Instant,
};

struct Gate {
    tasks: usize,
    open: AtomicBool,
    state: Mutex<GateState>,
}

struct GateState {
    tasks: Vec<Waker>,
    waiter: Option<Waker>,
}

impl Gate {
    fn new(tasks: usize) -> Arc<Self> {
        Arc::new(Self {
            tasks,
            open: AtomicBool::new(false),
            state: Mutex::new(GateState {
                tasks: Vec::with_capacity(tasks),
                waiter: None,
            }),
        })
    }

    fn register(&self, waker: &Waker) {
        let waiter = {
            let mut state = self.state.lock().unwrap();
            state.tasks.push(waker.clone());
            if state.tasks.len() == self.tasks {
                state.waiter.take()
            } else {
                None
            }
        };
        if let Some(waiter) = waiter {
            waiter.wake();
        }
    }

    async fn ready(&self) {
        poll_fn(|context| {
            let mut state = self.state.lock().unwrap();
            if state.tasks.len() == self.tasks {
                Poll::Ready(())
            } else {
                state.waiter = Some(context.waker().clone());
                Poll::Pending
            }
        })
        .await;
    }

    fn release(&self) {
        let tasks = std::mem::take(&mut self.state.lock().unwrap().tasks);
        assert_eq!(tasks.len(), self.tasks);
        self.open.store(true, Ordering::Release);
        for task in tasks {
            task.wake();
        }
    }
}

// Both executors poll this exact future. Its first poll parks at the untimed gate;
// the measured path touches only task-local state and calls wake_by_ref.
struct TaskPoll {
    gate: Option<Arc<Gate>>,
    registered: bool,
    remaining: usize,
    consume_wake: bool,
    polls: usize,
}

impl TaskPoll {
    fn new(gate: Arc<Gate>, polls: usize) -> Self {
        assert!(polls > 0);
        Self {
            gate: Some(gate),
            registered: false,
            remaining: polls,
            consume_wake: std::env::var("TASK_POLL_WAKE").as_deref() == Ok("owned"),
            polls,
        }
    }
}

impl Future for TaskPoll {
    type Output = usize;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if let Some(gate) = &this.gate {
            if !this.registered {
                gate.register(context.waker());
                this.registered = true;
                return Poll::Pending;
            }
            if !gate.open.load(Ordering::Acquire) {
                return Poll::Pending;
            }
            this.gate = None;
        }
        this.remaining -= 1;
        if this.remaining == 0 {
            Poll::Ready(this.polls)
        } else {
            if this.consume_wake {
                context.waker().clone().wake();
            } else {
                context.waker().wake_by_ref();
            }
            Poll::Pending
        }
    }
}

fn tasks(gate: &Arc<Gate>, total: usize) -> impl Iterator<Item = TaskPoll> + '_ {
    (0..gate.tasks).map(move |index| {
        let count = total / gate.tasks + usize::from(index < total % gate.tasks);
        TaskPoll::new(Arc::clone(gate), count)
    })
}

async fn measure<H, E>(gate: Arc<Gate>, handles: Vec<H>, total: usize) -> serde_json::Value
where
    H: Future<Output = Result<usize, E>>,
    E: Debug,
{
    gate.ready().await;
    let start = Instant::now();
    gate.release();
    let mut completed = 0;
    for handle in handles {
        completed += handle.await.unwrap();
    }
    let elapsed = start.elapsed();
    assert_eq!(completed, total);
    serde_json::json!({
        "polls": completed,
        "seconds": elapsed.as_secs_f64(),
        "polls_per_second": completed as f64 / elapsed.as_secs_f64(),
    })
}

#[cfg(target_os = "linux")]
struct NoIoSelector;

#[cfg(target_os = "linux")]
impl core_scheduler_native::Selector for NoIoSelector {
    fn close(&self) {}
    async fn shutdown(&self) {}
}

fn run(
    backend: &str,
    workers: usize,
    count: usize,
    polls: usize,
    samples: usize,
    warmup: usize,
) -> serde_json::Value {
    assert!(workers > 0 && count > 0 && polls >= count && samples > 0);
    let mut results = Vec::with_capacity(samples);
    match backend {
        "tokio" => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(workers)
                .build()
                .unwrap();
            for sample in 0..warmup + samples {
                let gate = Gate::new(count);
                let handles = tasks(&gate, polls)
                    .map(|task| runtime.spawn(task))
                    .collect();
                let result = runtime.block_on(measure(gate, handles, polls));
                if sample >= warmup {
                    results.push(result);
                }
            }
        }
        #[cfg(target_os = "linux")]
        "native" => {
            use core_scheduler::Spawn;
            let scheduler = core_scheduler_native::NativeScheduler::with_selector(
                NoIoSelector,
                core_scheduler_native::NativeExecutorOptions {
                    workers,
                    timer_capacity: 1,
                },
            )
            .unwrap();
            for sample in 0..warmup + samples {
                let gate = Gate::new(count);
                let handles = tasks(&gate, polls)
                    .map(|task| scheduler.spawn(task).unwrap())
                    .collect();
                let result = scheduler.block_on(measure(gate, handles, polls));
                if sample >= warmup {
                    results.push(result);
                }
            }
            scheduler.block_on(scheduler.shutdown());
        }
        _ => panic!("unsupported backend: {backend}"),
    }
    serde_json::json!({
        "backend": backend,
        "wake": std::env::var("TASK_POLL_WAKE").unwrap_or_else(|_| "borrowed".into()),
        "workers": workers,
        "tasks": count,
        "polls": polls,
        "warmup_samples": warmup,
        "samples": results,
    })
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let backend = args.get(1).map(String::as_str).unwrap_or("tokio");
    let number = |index: usize, fallback: usize| {
        args.get(index)
            .map(|value| value.parse().unwrap())
            .unwrap_or(fallback)
    };
    println!(
        "{}",
        run(
            backend,
            number(2, 1),
            number(3, 32),
            number(4, 10_000_000),
            number(5, 7),
            number(6, 1)
        )
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::atomic::AtomicUsize, task::Wake};

    struct WakeCount(AtomicUsize);
    impl Wake for WakeCount {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn initial_poll_is_excluded_and_every_pending_poll_wakes_once() {
        let gate = Gate::new(1);
        let mut task = TaskPoll::new(Arc::clone(&gate), 5);
        let wakes = Arc::new(WakeCount(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&wakes));
        let mut context = Context::from_waker(&waker);
        assert!(Pin::new(&mut task).poll(&mut context).is_pending());
        assert_eq!(task.remaining, 5);
        assert_eq!(wakes.0.load(Ordering::Relaxed), 0);
        gate.release();
        for remaining in (1..5).rev() {
            assert!(Pin::new(&mut task).poll(&mut context).is_pending());
            assert_eq!(task.remaining, remaining);
        }
        assert_eq!(Pin::new(&mut task).poll(&mut context), Poll::Ready(5));
        assert_eq!(wakes.0.load(Ordering::Relaxed), 5);
    }

    #[test]
    fn backends_complete_the_same_exact_poll_budget() {
        #[cfg(target_os = "linux")]
        let backends = ["tokio", "native"];
        #[cfg(not(target_os = "linux"))]
        let backends = ["tokio"];
        for backend in backends {
            for workers in [1, 4] {
                for count in [1, 3, 64] {
                    let result = run(backend, workers, count, 1009, 2, 1);
                    for sample in result["samples"].as_array().unwrap() {
                        assert_eq!(sample["polls"], 1009);
                        assert!(sample["polls_per_second"].as_f64().unwrap() > 0.0);
                    }
                }
            }
        }
    }
}
