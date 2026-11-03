// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

// Timers registered outside a worker must not depend on one particular worker returning from a long poll. Stall rescue fires them within about two milliseconds; `LATE` only distinguishes rescue from waiting for the hold, so loaded runners do not make these tests flaky.
use base_scheduler::{Cancellation, Clock, Spawn, Timers};
use base_scheduler_native::{NativeOptions, NativeScheduler, NativeTask};
use futures_lite::future::block_on;
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

const DELAY: Duration = Duration::from_millis(50);
// A long poll holds its worker for `HOLD`; a delayed timer would wait for most of it.
const HOLD: Duration = Duration::from_secs(5);
const LATE: Duration = Duration::from_millis(250);

// Occupy `busy` of the scheduler's workers with blocking polls until the returned sender is dropped.
fn occupy(scheduler: &NativeScheduler, busy: usize) -> (mpsc::Sender<()>, Vec<NativeTask<()>>) {
    occupy_for(scheduler, busy, HOLD)
}

fn occupy_for(
    scheduler: &NativeScheduler,
    busy: usize,
    hold: Duration,
) -> (mpsc::Sender<()>, Vec<NativeTask<()>>) {
    let (release, released) = mpsc::channel::<()>();
    let released = std::sync::Arc::new(std::sync::Mutex::new(released));
    let (started, start) = mpsc::channel();
    let tasks = (0..busy)
        .map(|_| {
            let released = std::sync::Arc::clone(&released);
            let started = started.clone();
            scheduler
                .spawn(async move {
                    started.send(()).unwrap();
                    // A blocking call stands in for any long synchronous poll.
                    let deadline = Instant::now() + hold;
                    while Instant::now() < deadline {
                        if released
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_millis(10))
                            .is_err_and(|error| error == mpsc::RecvTimeoutError::Disconnected)
                        {
                            break;
                        }
                    }
                })
                .unwrap()
        })
        .collect();
    for _ in 0..busy {
        start.recv_timeout(hold).unwrap();
    }
    (release, tasks)
}

fn assert_prompt(label: &str, elapsed: Duration) {
    assert!(
        elapsed >= DELAY,
        "{label} completed early after {elapsed:?}"
    );
    assert!(
        elapsed < LATE,
        "{label} waited {elapsed:?} for a {DELAY:?} deadline"
    );
}

#[test]
fn root_sleeps_fire_while_a_worker_is_blocked() {
    for (workers, busy) in [(2, 1), (4, 3)] {
        let scheduler = NativeScheduler::with_options(NativeOptions {
            workers,
            ..Default::default()
        })
        .unwrap();
        let (release, tasks) = occupy(&scheduler, busy);
        #[cfg(feature = "diagnostics")]
        assert_eq!(scheduler.rescue_statistics().timers, 0);
        let never = Cancellation::never();
        // Several sleeps cover every worker a registration could have selected.
        for index in 0..workers * 2 {
            let start = Instant::now();
            block_on(scheduler.sleep(DELAY, &never)).unwrap();
            assert_prompt(&format!("root sleep {index}"), start.elapsed());
        }
        let mut timer = scheduler.timer();
        for index in 0..workers * 2 {
            let start = Instant::now();
            block_on(scheduler.timer_wait_until(&mut timer, Instant::now() + DELAY, &never))
                .unwrap();
            assert_prompt(&format!("root timer {index}"), start.elapsed());
        }
        // Some registrations landed on a blocked worker and were rescued while it was inside one poll.
        #[cfg(feature = "diagnostics")]
        {
            let statistics = scheduler.rescue_statistics();
            assert!(statistics.timers > 0, "{statistics:?}");
            assert!(statistics.stuck_polls > 0, "{statistics:?}");
        }
        drop(release);
        for task in tasks {
            block_on(task).unwrap();
        }
        block_on(scheduler.shutdown());
    }
}

#[test]
fn concurrent_external_sleeps_fire_while_a_worker_is_blocked() {
    let scheduler = NativeScheduler::with_options(NativeOptions {
        workers: 2,
        ..Default::default()
    })
    .unwrap();
    let (release, tasks) = occupy(&scheduler, 1);
    std::thread::scope(|scope| {
        let threads: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    let start = Instant::now();
                    block_on(scheduler.sleep(DELAY, &Cancellation::never())).unwrap();
                    start.elapsed()
                })
            })
            .collect();
        for (index, thread) in threads.into_iter().enumerate() {
            assert_prompt(&format!("external sleep {index}"), thread.join().unwrap());
        }
    });
    drop(release);
    for task in tasks {
        block_on(task).unwrap();
    }
    block_on(scheduler.shutdown());
}

#[test]
fn pending_root_deadline_moves_off_a_worker_that_starts_a_long_poll() {
    let scheduler = NativeScheduler::with_options(NativeOptions {
        workers: 2,
        ..Default::default()
    })
    .unwrap();
    // Admission alternates between workers, so either may be watching the deadline when its long poll starts.
    for index in 0..4 {
        std::thread::scope(|scope| {
            let sleeper = scope.spawn(|| {
                let start = Instant::now();
                block_on(scheduler.sleep(DELAY * 4, &Cancellation::never())).unwrap();
                start.elapsed()
            });
            // Let an idle worker adopt the registered deadline before the poll begins.
            std::thread::sleep(DELAY);
            let (release, tasks) = occupy(&scheduler, 1);
            let elapsed = sleeper.join().unwrap();
            assert!(
                elapsed >= DELAY * 4 && elapsed < LATE,
                "root sleep {index} waited {elapsed:?} for a {:?} deadline",
                DELAY * 4
            );
            drop(release);
            for task in tasks {
                block_on(task).unwrap();
            }
        });
    }
    block_on(scheduler.shutdown());
}

#[test]
fn root_sleeps_wait_for_a_blocked_worker_when_rescue_is_disabled() {
    let hold = Duration::from_millis(500);
    let scheduler = NativeScheduler::with_options(NativeOptions {
        workers: 1,
        stall_rescue: false,
        ..Default::default()
    })
    .unwrap();
    let (release, tasks) = occupy_for(&scheduler, 1, hold);
    let start = Instant::now();
    block_on(scheduler.sleep(DELAY, &Cancellation::never())).unwrap();
    // The only worker collects its own timers once its long poll returns.
    assert!(
        start.elapsed() >= hold / 2,
        "rescue fired a timer after {:?} although it is disabled",
        start.elapsed()
    );
    drop(release);
    for task in tasks {
        block_on(task).unwrap();
    }
    block_on(scheduler.shutdown());
}

#[test]
fn root_sleeps_fire_when_no_worker_is_idle() {
    let scheduler = NativeScheduler::with_options(NativeOptions {
        workers: 1,
        ..Default::default()
    })
    .unwrap();
    let (release, tasks) = occupy(&scheduler, 1);
    for index in 0..4 {
        let start = Instant::now();
        block_on(scheduler.sleep(DELAY, &Cancellation::never())).unwrap();
        assert_prompt(&format!("root sleep {index}"), start.elapsed());
    }
    drop(release);
    for task in tasks {
        block_on(task).unwrap();
    }
    block_on(scheduler.shutdown());
}
