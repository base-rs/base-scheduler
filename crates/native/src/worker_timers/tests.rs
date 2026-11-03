// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::{NativeOptions, NativeScheduler, test_support};
use base_scheduler::Spawn;
use base_scheduler_conformance::ClockFixture;
use futures_channel::oneshot;
use futures_lite::future::{block_on, poll_once};
use std::task::{Context, Wake};

// Collect expired timers from the calling thread, then count what remains.
fn inspect(shared: &Arc<Shared>, owner: usize) -> usize {
    expire(shared, owner, &mut buffer(), false);
    len(shared, owner)
}

impl ClockFixture for NativeScheduler {
    type Scheduler = Self;
    fn scheduler(&self) -> &Self {
        self
    }
    fn advance(&self, duration: Duration) {
        *self.tasks.shared.paused.lock().unwrap().as_mut().unwrap() += duration;
        self.registrations();
    }
    fn registrations(&self) -> usize {
        (0..self.worker_count())
            .map(|owner| inspect(&self.tasks.shared, owner))
            .sum()
    }
}

fn run(test: impl AsyncFnOnce(&NativeScheduler) + Send + 'static) {
    test_support::run(move || {
        let scheduler = test_support::scheduler();
        *scheduler.tasks.shared.paused.lock().unwrap() = Some(Instant::now());
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| block_on(test(&scheduler))));
        block_on(scheduler.shutdown());
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    });
}

mod conformance {
    base_scheduler_conformance::clock_tests!(run: super::run);
}

mod worker_conformance {
    use super::*;
    macro_rules! case {
        ($($name:ident),* $(,)?) => {$(
            #[test]
            fn $name() {
                test_support::run(|| {
                    let scheduler = Arc::new(test_support::scheduler());
                    *scheduler.tasks.shared.paused.lock().unwrap() = Some(Instant::now());
                    let owned = Arc::clone(&scheduler);
                    let task = scheduler.spawn(async move {
                        base_scheduler_conformance::clock::$name(&*owned).await;
                    }).unwrap();
                    let result = block_on(task);
                    block_on(scheduler.shutdown());
                    result.unwrap();
                });
            }
        )*};
    }
    case!(
        registration,
        pre_cancelled,
        deadline,
        relative,
        cancellation,
        expiry_wins,
        overflow,
        timeout
    );
}

#[test]
fn worker_sleep_registration_repoll_and_drop_allocate_nothing() {
    test_support::run(|| {
        let scheduler = Arc::new(test_support::scheduler());
        let owned = Arc::clone(&scheduler);
        let task = scheduler
            .spawn(async move {
                let token = Cancellation::new();
                let deadline = owned.now() + Duration::from_secs(3600);
                let allocations = allocation_counter::measure(|| {
                    for _ in 0..32 {
                        let mut sleep = pin!(owned.sleep_until(deadline, &token));
                        for _ in 0..2 {
                            assert!(
                                sleep
                                    .as_mut()
                                    .poll(&mut Context::from_waker(Waker::noop()))
                                    .is_pending()
                            );
                        }
                    }
                });
                assert_eq!(allocations.count_total, 0, "{allocations:?}");
                assert_eq!(owned.registrations(), 0);
            })
            .unwrap();
        block_on(task).unwrap();
        block_on(scheduler.shutdown());
    });
}

#[test]
fn earlier_external_deadline_interrupts_worker_wait() {
    test_support::run(|| {
        let scheduler = test_support::scheduler();
        let never = Cancellation::never();
        let mut distant = Box::pin(scheduler.sleep(Duration::from_secs(3600), &never));
        assert!(block_on(poll_once(&mut distant)).is_none());
        assert_eq!(scheduler.registrations(), 1);
        let deadline = scheduler.now() + Duration::from_millis(10);
        scheduler
            .block_on(scheduler.sleep_until(deadline, &never))
            .unwrap();
        assert!(scheduler.now() >= deadline);
        drop(distant);
        assert_eq!(scheduler.registrations(), 0);
        block_on(scheduler.shutdown());
    });
}

#[test]
fn full_capacity_reclaims_cancelled_timers_and_shutdown_rejects_waits() {
    test_support::run(|| {
        let scheduler = NativeScheduler::with_options(NativeOptions {
            workers: 1,
            timer_capacity: 1,
            ..Default::default()
        })
        .unwrap();
        let never = Cancellation::never();
        let deadline = scheduler.now() + Duration::from_secs(3600);
        // The timer engine can round reservations up to its minimum batch size.
        let capacity =
            base_scheduler_timers::local::TimerQueue::<Waker>::with_capacity(1).capacity();
        let mut waits: Vec<_> = (0..capacity)
            .map(|_| Box::pin(scheduler.sleep_until(deadline, &never)))
            .collect();
        for wait in &mut waits {
            assert!(block_on(poll_once(wait)).is_none());
        }
        assert_eq!(scheduler.registrations(), capacity);
        assert_eq!(
            block_on(scheduler.sleep_until(deadline, &never))
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        drop(waits);
        assert_eq!(scheduler.registrations(), 0);
        let mut next = Box::pin(scheduler.sleep_until(deadline, &never));
        assert!(block_on(poll_once(&mut next)).is_none());
        assert_eq!(scheduler.registrations(), 1);
        scheduler.close();
        assert_eq!(
            block_on(next).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(
            block_on(scheduler.sleep_until(deadline, &never))
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
        block_on(scheduler.shutdown());
    });
}

#[test]
fn external_registration_and_release_reach_a_blocked_owner_immediately() {
    test_support::run(|| {
        let scheduler = test_support::scheduler();
        let (gate, started, release) = test_support::Gate::new();
        let task = scheduler
            .spawn(async move {
                gate.wait();
            })
            .unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let never = Cancellation::never();
        let mut sleep = Box::pin(scheduler.sleep(Duration::from_secs(3600), &never));
        assert!(block_on(poll_once(&mut sleep)).is_none());
        // The owner is still blocked, so neither change can have passed through it.
        assert_eq!(len(&scheduler.tasks.shared, 0), 1);
        drop(sleep);
        assert_eq!(len(&scheduler.tasks.shared, 0), 0);
        drop(release);
        block_on(task).unwrap();
        block_on(scheduler.shutdown());
    });
}

#[derive(Default)]
struct Probe(std::sync::atomic::AtomicUsize);
impl Wake for Probe {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn external_update_installs_the_replacement_waker_while_the_owner_is_blocked() {
    test_support::run(|| {
        let scheduler = test_support::scheduler_with_rescue(false);
        let (gate, started, release) = test_support::Gate::new();
        let task = scheduler
            .spawn(async move {
                gate.wait();
            })
            .unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let never = Cancellation::never();
        let mut sleep = Box::pin(scheduler.sleep(Duration::from_millis(20), &never));
        assert!(
            sleep
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        let probe = Arc::new(Probe::default());
        let waker = Waker::from(Arc::clone(&probe));
        assert!(
            sleep
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        std::thread::sleep(Duration::from_millis(30));
        // Expiry on any thread observes the update, although the owner never ran.
        assert_eq!(expire(&scheduler.tasks.shared, 0, &mut buffer(), true), 1);
        assert_eq!(probe.0.load(Ordering::SeqCst), 1);
        drop(release);
        block_on(task).unwrap();
        block_on(sleep).unwrap();
        block_on(scheduler.shutdown());
    });
}

#[test]
fn moving_a_sleep_to_another_worker_updates_and_releases_its_original_queue() {
    test_support::run(|| {
        let scheduler = Arc::new(
            NativeScheduler::with_options(NativeOptions {
                workers: 2,
                ..Default::default()
            })
            .unwrap(),
        );
        let (gate, started, release) = test_support::Gate::new();
        let (send, receive) = oneshot::channel();
        let token = Cancellation::new();
        let cancel = token.clone();
        let owned = Arc::clone(&scheduler);
        let first = scheduler
            .spawn(async move {
                let owner = worker::current(&owned.tasks.shared).unwrap();
                let mut sleep =
                    Box::pin(async move { owned.sleep(Duration::from_secs(3600), &token).await });
                assert!(
                    sleep
                        .as_mut()
                        .poll(&mut Context::from_waker(Waker::noop()))
                        .is_pending()
                );
                assert!(send.send((owner, sleep)).is_ok());
                // Keep the original worker occupied so the next task must run elsewhere.
                gate.wait();
            })
            .unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let (owner, mut sleep) = block_on(receive).unwrap();
        let owned = Arc::clone(&scheduler);
        let second = scheduler
            .spawn(async move {
                let current = worker::current(&owned.tasks.shared).unwrap();
                assert_ne!(current, owner);
                let probe = Waker::from(Arc::new(Probe::default()));
                assert!(
                    sleep
                        .as_mut()
                        .poll(&mut Context::from_waker(&probe))
                        .is_pending()
                );
                cancel.cancel();
                assert!(matches!(
                    sleep.await,
                    Err(base_scheduler::OperationError::Cancelled(_))
                ));
                // Cancellation released the key in the original queue while its owner is still blocked.
                assert_eq!(len(&owned.tasks.shared, owner), 0);
            })
            .unwrap();
        block_on(second).unwrap();
        drop(release);
        block_on(first).unwrap();
        assert_eq!(scheduler.registrations(), 0);
        block_on(scheduler.shutdown());
    });
}

#[test]
fn calls_from_another_scheduler_use_the_explicitly_supplied_scheduler() {
    test_support::run(|| {
        let owner = Arc::new(test_support::scheduler());
        let other = test_support::scheduler();
        let owned = Arc::clone(&owner);
        let task = other
            .spawn(async move {
                owned
                    .sleep(Duration::from_millis(1), &Cancellation::never())
                    .await
                    .unwrap();
            })
            .unwrap();
        block_on(task).unwrap();
        block_on(other.shutdown());
        block_on(owner.shutdown());
    });
}

#[test]
fn timer_wakers_can_reenter_and_panic_without_stopping_workers() {
    test_support::run(|| {
        struct Reentrant(Arc<NativeScheduler>);
        impl Wake for Reentrant {
            fn wake(self: Arc<Self>) {
                // This uses the same queue and would fail if the wake retained a TLS borrow.
                let never = Cancellation::never();
                let mut sleep = pin!(self.0.sleep(Duration::from_secs(3600), &never));
                assert!(
                    sleep
                        .as_mut()
                        .poll(&mut Context::from_waker(Waker::noop()))
                        .is_pending()
                );
                panic!("timer waker panic");
            }
        }
        let scheduler = Arc::new(test_support::scheduler());
        let never = Cancellation::never();
        let mut sleep = Box::pin(scheduler.sleep(Duration::from_millis(10), &never));
        assert!(
            sleep
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        assert_eq!(scheduler.registrations(), 1);
        let waker = Waker::from(Arc::new(Reentrant(Arc::clone(&scheduler))));
        assert!(
            sleep
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        block_on(sleep).unwrap();
        assert_eq!(
            block_on(scheduler.spawn(async { 42 }).unwrap()).unwrap(),
            42
        );
        block_on(scheduler.shutdown());
    });
}

#[test]
fn local_capacity_and_delayed_updates_obey_registration_ownership() {
    test_support::run(|| {
        let scheduler = Arc::new(
            NativeScheduler::with_options(NativeOptions {
                workers: 1,
                timer_capacity: 1,
                ..Default::default()
            })
            .unwrap(),
        );
        let owned = Arc::clone(&scheduler);
        let task = scheduler
            .spawn(async move {
                let shared = &owned.tasks.shared;
                let never = Cancellation::never();
                let deadline = owned.now() + Duration::from_secs(3600);
                let owner = worker::current(shared).unwrap();
                let capacity = lock(&shared.workers[owner]).capacity();
                let mut waits: Vec<_> = (0..capacity)
                    .map(|_| Box::pin(owned.sleep_until(deadline, &never)))
                    .collect();
                for wait in &mut waits {
                    assert!(poll_once(wait).await.is_none());
                }
                assert_eq!(
                    owned
                        .sleep_until(deadline, &never)
                        .await
                        .unwrap_err()
                        .kind(),
                    io::ErrorKind::WouldBlock
                );
                drop(waits);
                // A migrated update can arrive after expiry but before the waiter releases its key.
                let registration = register(shared, owned.now(), Waker::noop().clone()).unwrap();
                assert_eq!(expire(shared, owner, &mut buffer(), false), 1);
                let probe = Arc::new(Probe::default());
                registration.update(Waker::from(Arc::clone(&probe)));
                assert_eq!(probe.0.load(Ordering::SeqCst), 1);
                drop(registration);
                assert_eq!(owned.registrations(), 0);
            })
            .unwrap();
        block_on(task).unwrap();
        block_on(scheduler.shutdown());
    });
}

#[test]
fn closure_rejects_registrations_and_notifies_late_updates() {
    test_support::run(|| {
        let scheduler = Arc::new(test_support::scheduler());
        let shared = &scheduler.tasks.shared;
        let registration = register(
            shared,
            scheduler.now() + Duration::from_secs(3600),
            Waker::noop().clone(),
        )
        .unwrap();
        scheduler.close();
        assert_eq!(
            register(
                shared,
                scheduler.now() + Duration::from_secs(3600),
                Waker::noop().clone()
            )
            .err()
            .unwrap()
            .kind(),
            io::ErrorKind::BrokenPipe
        );
        block_on(scheduler.shutdown());
        let probe = Arc::new(Probe::default());
        registration.update(Waker::from(Arc::clone(&probe)));
        assert_eq!(probe.0.load(Ordering::SeqCst), 1);
        drop(registration);
        assert_eq!(scheduler.registrations(), 0);
        // The registration helper handles closure between the public precheck and the queue lock.
        assert_eq!(
            block_on(wait(shared, scheduler.now() + Duration::from_secs(1)))
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
    });
}

#[test]
fn expiry_passes_collect_more_than_one_batch_and_rescuers_skip_a_locked_queue() {
    test_support::run(|| {
        let scheduler = test_support::scheduler_with_rescue(false);
        let shared = &scheduler.tasks.shared;
        // Hold the only worker, so that only this thread collects expired timers.
        let (gate, started, release) = test_support::Gate::new();
        let task = scheduler
            .spawn(async move {
                gate.wait();
            })
            .unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let probe = Arc::new(Probe::default());
        let waker = Waker::from(Arc::clone(&probe));
        let count = BATCH * 2 + 1;
        // Every deadline precedes the epoch and publishes as zero, so only the first registration lowers it.
        let base = scheduler.now();
        let registrations: Vec<_> = (0..count)
            .rev()
            .map(|index| {
                register(
                    shared,
                    base - Duration::from_secs(10) + Duration::from_millis(index as u64),
                    waker.clone(),
                )
                .unwrap()
            })
            .collect();
        {
            let _held = lock(&shared.workers[0]);
            assert_eq!(expire(shared, 0, &mut buffer(), true), 0);
        }
        assert_eq!(expire(shared, 0, &mut buffer(), true), count);
        assert_eq!(probe.0.load(Ordering::SeqCst), count);
        // An empty queue publishes no deadline, so later passes return without reading the clock.
        assert_eq!(
            shared.workers[0].deadline.load(Ordering::Relaxed),
            NO_DEADLINE
        );
        assert_eq!(expire(shared, 0, &mut buffer(), true), 0);
        drop(registrations);
        let distant: Vec<_> = (0..count)
            .map(|_| register(shared, base + Duration::from_secs(3600), waker.clone()).unwrap())
            .collect();
        assert_eq!(expire(shared, 0, &mut buffer(), true), 0);
        scheduler.close();
        drop(release);
        block_on(task).unwrap();
        block_on(scheduler.shutdown());
        assert_eq!(len(shared, 0), 0);
        assert_eq!(probe.0.load(Ordering::SeqCst), count * 2);
        drop(distant);
    });
}

#[test]
fn shutdown_waits_for_timer_callbacks_and_live_waits_can_outlast_workers() {
    test_support::run(|| {
        struct GatedWake(Arc<test_support::Gate>);
        impl Wake for GatedWake {
            fn wake(self: Arc<Self>) {
                self.0.wait();
            }
        }
        let scheduler = test_support::scheduler();
        let (gate, started, release) = test_support::Gate::new();
        let waker = Waker::from(Arc::new(GatedWake(gate)));
        let never = Cancellation::never();
        let mut sleep = Box::pin(scheduler.sleep(Duration::from_secs(3600), &never));
        assert!(
            sleep
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        assert_eq!(scheduler.registrations(), 1);
        assert!(
            sleep
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        assert_eq!(scheduler.registrations(), 1);
        scheduler.close();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut shutdown = Box::pin(scheduler.shutdown());
        assert!(block_on(poll_once(&mut shutdown)).is_none());
        drop(release);
        block_on(shutdown);
        assert_eq!(
            block_on(sleep).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    });
}
