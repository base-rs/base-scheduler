// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use base_benchmark::{ResourceSource, ResourceUsage};
use std::{
    cell::RefCell,
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

// Turns a hang into a failure. It bounds no expected speed, so it is far longer than any test needs.
const WATCHDOG: Duration = Duration::from_secs(60);

thread_local! {
    // The step a test running under `run` on this thread last named.
    static STEP: RefCell<Option<Arc<Mutex<String>>>> = const { RefCell::new(None) };
}

/// Run a test that waits on schedulers, sockets or threads on a thread of its own, so that a hang fails it at the watchdog, naming the test and the last step it reached.
pub(crate) fn run(test: impl FnOnce() + Send + 'static) {
    let name = std::thread::current()
        .name()
        .unwrap_or("unnamed test")
        .to_owned();
    let step = Arc::new(Mutex::new(String::from("running")));
    let current = Arc::clone(&step);
    let (done, completion) = mpsc::channel();
    std::thread::Builder::new()
        .name(name.clone())
        .spawn(move || {
            STEP.set(Some(current));
            let _ = done.send(catch_unwind(AssertUnwindSafe(test)));
        })
        .unwrap();
    watch(&name, &completion, &step, WATCHDOG);
}

// Wait for the test's outcome, naming the test and its last step if the watchdog expires first.
fn watch(
    name: &str,
    completion: &mpsc::Receiver<std::thread::Result<()>>,
    step: &Mutex<String>,
    watchdog: Duration,
) {
    match completion.recv_timeout(watchdog) {
        Ok(result) => {
            if let Err(payload) = result {
                std::panic::resume_unwind(payload);
            }
        }
        Err(_) => panic!(
            "{name}: test watchdog expired after {watchdog:?} while {}",
            step.lock().unwrap()
        ),
    }
}

/// Name what the test running under `run` on this thread is about to wait for, so that a hang reaching the watchdog reports where it stopped.
pub(crate) fn step(label: impl Into<String>) {
    STEP.with_borrow(|step| {
        *step
            .as_ref()
            .expect("steps are named by a test running under `run`")
            .lock()
            .unwrap() = label.into();
    });
}

/// Resource usage that advances at every snapshot, so that tests measure on every platform, or that fails after a number of snapshots.
#[derive(Default)]
pub(crate) struct TestSource {
    remaining: Option<usize>,
    usage: ResourceUsage,
}

impl TestSource {
    /// A source whose first snapshot fails, so no interval starts.
    pub(crate) fn failing() -> Self {
        Self::failing_after(0)
    }

    /// A source that fails after `snapshots` successful ones: after one, an interval starts but cannot finish.
    pub(crate) fn failing_after(snapshots: usize) -> Self {
        Self {
            remaining: Some(snapshots),
            ..Self::default()
        }
    }
}

impl ResourceSource for TestSource {
    fn snapshot(&mut self) -> io::Result<ResourceUsage> {
        if let Some(remaining) = &mut self.remaining {
            if *remaining == 0 {
                return Err(io::Error::other("no resource usage in this test"));
            }
            *remaining -= 1;
        }
        self.usage.user_cpu_ns += 1_000;
        self.usage.voluntary_context_switches += 1;
        Ok(self.usage)
    }

    fn resolution_ns(&self) -> u64 {
        1
    }
}

#[test]
fn watchdog_propagates_assertion_panics() {
    assert!(catch_unwind(|| run(|| panic!("test assertion"))).is_err());
}

#[test]
fn watchdog_names_the_test_and_the_step_it_reached() {
    let (outcome, completion) = mpsc::channel();
    let last = Mutex::new(String::from("waiting for a release"));
    let payload = catch_unwind(AssertUnwindSafe(|| {
        watch("a hung test", &completion, &last, Duration::ZERO)
    }))
    .unwrap_err();
    drop(outcome);
    let message = payload.downcast_ref::<String>().unwrap();
    assert!(message.starts_with("a hung test: "), "{message}");
    assert!(
        message.ends_with("while waiting for a release"),
        "{message}"
    );
    // Steps name what the test thread under `run` waits for, and that thread carries the test's name.
    let name = std::thread::current().name().map(str::to_owned);
    run(move || {
        assert_eq!(std::thread::current().name().map(str::to_owned), name);
        step("checking a step");
        STEP.with_borrow(|step| {
            assert_eq!(*step.as_ref().unwrap().lock().unwrap(), "checking a step");
        });
    });
}
