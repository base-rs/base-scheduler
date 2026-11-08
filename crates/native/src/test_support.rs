// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::{
    cell::RefCell,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

// Turns a hang into a failure. It bounds no expected speed, so it is far longer than any test needs.
const WATCHDOG: Duration = Duration::from_secs(15);

thread_local! {
    // The step a test running under `run` on this thread last named.
    static STEP: RefCell<Option<Arc<Mutex<String>>>> = const { RefCell::new(None) };
}

pub(crate) fn run(test: impl FnOnce() + Send + 'static) {
    let step = Arc::new(Mutex::new(String::from("starting")));
    let current = Arc::clone(&step);
    let (done, completion) = mpsc::channel();
    std::thread::spawn(move || {
        STEP.set(Some(current));
        let _ = done.send(catch_unwind(AssertUnwindSafe(test)));
    });
    watch(&completion, &step, WATCHDOG);
}

// Wait for the test's outcome, naming its last step if the watchdog expires first.
fn watch(
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
            "native test watchdog expired after {watchdog:?} while {}",
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
pub(crate) fn scheduler() -> crate::NativeScheduler {
    scheduler_with_rescue(true)
}
// Without stall rescue, a test that blocks the only worker controls when its timers are collected.
pub(crate) fn scheduler_with_rescue(stall_rescue: bool) -> crate::NativeScheduler {
    crate::NativeScheduler::with_options(crate::NativeOptions {
        workers: 1,
        threads: 1,
        capacity: 4,
        stall_rescue,
        ..Default::default()
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

#[test]
fn watchdog_names_the_step_a_hung_test_reached() {
    let (outcome, completion) = mpsc::channel();
    let last = Mutex::new(String::from("waiting for a release"));
    let payload = catch_unwind(AssertUnwindSafe(|| {
        watch(&completion, &last, Duration::ZERO)
    }))
    .unwrap_err();
    drop(outcome);
    let message = payload.downcast_ref::<String>().unwrap();
    assert!(
        message.ends_with("while waiting for a release"),
        "{message}"
    );
    // Steps name what the test thread under `run` waits for.
    run(|| {
        step("checking a step");
        STEP.with_borrow(|step| {
            assert_eq!(*step.as_ref().unwrap().lock().unwrap(), "checking a step");
        });
    });
}
