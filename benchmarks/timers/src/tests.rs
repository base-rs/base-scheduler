// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use core_scheduler::{Clock, OperationError};

async fn semantics<S: TimerClock>(scheduler: &S) {
    let never = Cancellation::never();
    let mut timer = S::Timer::default();
    let far = scheduler.now() + Duration::from_secs(30);
    pending_once(scheduler.timer_wait(&mut timer, far, &never)).await;
    // Pull a pending timer forward, then reuse it after expiry.
    let early = scheduler.now() + Duration::from_millis(2);
    scheduler
        .timer_wait(&mut timer, early, &never)
        .await
        .unwrap();
    assert!(scheduler.now() >= early);
    let later = scheduler.now() + Duration::from_millis(2);
    scheduler
        .timer_wait(&mut timer, later, &never)
        .await
        .unwrap();
    assert!(scheduler.now() >= later);
    // Cancellation retires a previously armed wait, without poisoning the resource.
    {
        let cancellation = Cancellation::new();
        let mut waiting = pin!(scheduler.timer_wait(&mut timer, far, &cancellation));
        assert!(
            futures_lite::future::poll_once(waiting.as_mut())
                .await
                .is_none()
        );
        cancellation.cancel();
        assert!(matches!(waiting.await, Err(OperationError::Cancelled(_))));
    }
    let cancelled = Cancellation::new();
    cancelled.cancel();
    assert!(
        scheduler
            .timer_wait(&mut S::Timer::default(), scheduler.now(), &cancelled)
            .await
            .is_err()
    );
    scheduler
        .timer_wait(&mut timer, scheduler.now(), &never)
        .await
        .unwrap();
    // After must not complete at its old, earlier deadline when extended.
    let mut after = After::<S::Timer>::default();
    let first = scheduler.now() + Duration::from_millis(2);
    pending_once(after.wait(scheduler, first, &never)).await;
    let extended = scheduler.now() + Duration::from_millis(4);
    after.wait(scheduler, extended, &never).await.unwrap();
    assert!(scheduler.now() >= extended);
    pending_once(after.wait(scheduler, far, &never)).await;
    after
        .wait(scheduler, scheduler.now(), &never)
        .await
        .unwrap();
    {
        let cancellation = Cancellation::new();
        let mut waiting = pin!(after.wait(scheduler, far, &cancellation));
        assert!(
            futures_lite::future::poll_once(waiting.as_mut())
                .await
                .is_none()
        );
        cancellation.cancel();
        assert!(waiting.await.is_err());
    }
    let cancelled = Cancellation::new();
    cancelled.cancel();
    assert!(after.wait(scheduler, far, &cancelled).await.is_err());
}

#[test]
fn direct_semantics() {
    let scheduler = core_scheduler_direct::DirectScheduler::new(Default::default()).unwrap();
    futures_lite::future::block_on(semantics(&scheduler));
    futures_lite::future::block_on(scheduler.shutdown());
}
#[test]
fn native_external_and_worker_semantics() {
    let scheduler = Arc::new(
        core_scheduler_native::NativeScheduler::with_options(
            core_scheduler_native::NativeOptions {
                workers: 2,
                ..Default::default()
            },
        )
        .unwrap(),
    );
    scheduler.block_on(semantics(&*scheduler));
    let instance = scheduler.clone();
    scheduler
        .block_on(
            scheduler
                .spawn(async move { semantics(&*instance).await })
                .unwrap(),
        )
        .unwrap();
    scheduler.block_on(scheduler.shutdown());
}
#[test]
fn tokio_semantics() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let scheduler = core_scheduler_tokio::TokioScheduler::new(runtime.handle().clone());
    runtime.block_on(semantics(&scheduler));
    runtime.block_on(scheduler.shutdown());
}

#[tokio::test(start_paused = true)]
async fn deferred_deadline_and_waker_replacement() {
    let scheduler = core_scheduler_tokio::TokioScheduler::new(tokio::runtime::Handle::current());
    let never = Cancellation::never();
    let mut after = After::default();
    let first = scheduler.now() + Duration::from_secs(1);
    pending_once(after.wait(&scheduler, first, &never)).await;
    let final_deadline = first + Duration::from_secs(1);
    tokio::time::advance(Duration::from_secs(1)).await;
    // A past armed deadline must rearm for the latest desired deadline.
    pending_once(after.wait(&scheduler, final_deadline, &never)).await;
    let mut waiting = pin!(after.wait(&scheduler, final_deadline, &never));
    assert!(
        futures_lite::future::poll_once(waiting.as_mut())
            .await
            .is_none()
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    waiting.await.unwrap();
    assert!(scheduler.now() >= final_deadline);
}

async fn incompatible<S: TimerClock>(first: &S, second: &S) {
    let never = Cancellation::never();
    let mut timer = S::Timer::default();
    pending_once(first.timer_wait(&mut timer, first.now() + Duration::from_secs(30), &never)).await;
    let result = second.timer_wait(&mut timer, second.now(), &never).await;
    assert!(
        matches!(result, Err(OperationError::Io(error)) if error.kind() == std::io::ErrorKind::InvalidInput)
    );
}
#[test]
fn rejects_incompatible_schedulers() {
    let one = core_scheduler_direct::DirectScheduler::new(Default::default()).unwrap();
    let two = core_scheduler_direct::DirectScheduler::new(Default::default()).unwrap();
    futures_lite::future::block_on(incompatible(&one, &two));
    let one = core_scheduler_native::NativeScheduler::with_options(
        core_scheduler_native::NativeOptions {
            workers: 1,
            ..Default::default()
        },
    )
    .unwrap();
    let two = core_scheduler_native::NativeScheduler::with_options(
        core_scheduler_native::NativeOptions {
            workers: 1,
            ..Default::default()
        },
    )
    .unwrap();
    one.block_on(incompatible(&one, &two));
    one.block_on(one.shutdown());
    two.block_on(two.shutdown());
    let rt1 = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let rt2 = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let one = core_scheduler_tokio::TokioScheduler::new(rt1.handle().clone());
    let two = core_scheduler_tokio::TokioScheduler::new(rt2.handle().clone());
    rt1.block_on(incompatible(&one, &two));
}

#[test]
fn native_resource_moves_between_tasks_and_executors() {
    let scheduler = Arc::new(
        core_scheduler_native::NativeScheduler::with_options(
            core_scheduler_native::NativeOptions {
                workers: 2,
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let instance = scheduler.clone();
    let timer = scheduler
        .block_on(
            scheduler
                .spawn(async move {
                    let mut timer = core_scheduler_native::NativeTimer::default();
                    let deadline = instance.now() + Duration::from_millis(50);
                    pending_once(instance.timer_wait(&mut timer, deadline, &Cancellation::never()))
                        .await;
                    (timer, deadline)
                })
                .unwrap(),
        )
        .unwrap();
    let instance = scheduler.clone();
    // Poll from another executor/thread; update and release still target the original owner.
    std::thread::spawn(move || {
        futures_lite::future::block_on(async move {
            let (mut timer, deadline) = timer;
            instance
                .timer_wait(&mut timer, deadline, &Cancellation::never())
                .await
                .unwrap();
        })
    })
    .join()
    .unwrap();
    scheduler.block_on(scheduler.shutdown());
}

#[test]
fn native_capacity_and_resource_drop() {
    let scheduler = core_scheduler_native::NativeScheduler::with_options(
        core_scheduler_native::NativeOptions {
            workers: 1,
            timer_capacity: 1,
            ..Default::default()
        },
    )
    .unwrap();
    scheduler.block_on(async {
        let never = Cancellation::never();
        let far = scheduler.now() + Duration::from_secs(30);
        let capacity =
            core_scheduler_timers::local::TimerQueue::<std::task::Waker>::with_capacity(1)
                .capacity();
        let mut first: Vec<_> = (0..capacity)
            .map(|_| core_scheduler_native::NativeTimer::default())
            .collect();
        let mut second = core_scheduler_native::NativeTimer::default();
        for timer in &mut first {
            pending_once(scheduler.timer_wait(timer, far, &never)).await;
        }
        assert_eq!(
            scheduler
                .timer_wait(&mut second, far, &never)
                .await
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
        drop(first);
        pending_once(scheduler.timer_wait(&mut second, far, &never)).await;
        scheduler.close();
        assert_eq!(
            scheduler
                .timer_wait(&mut second, far, &never)
                .await
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::BrokenPipe
        );
    });
    scheduler.block_on(scheduler.shutdown());
}

#[test]
fn closed_backends_reject_pending_timers() {
    let direct = core_scheduler_direct::DirectScheduler::new(Default::default()).unwrap();
    direct.close();
    futures_lite::future::block_on(async {
        assert_eq!(
            direct
                .timer_wait(
                    &mut Default::default(),
                    direct.now() + Duration::from_secs(1),
                    &Cancellation::never()
                )
                .await
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::BrokenPipe
        );
        direct.shutdown().await;
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let tokio = core_scheduler_tokio::TokioScheduler::new(runtime.handle().clone());
    tokio.close();
    runtime.block_on(async {
        assert_eq!(
            tokio
                .timer_wait(
                    &mut Default::default(),
                    tokio.now() + Duration::from_secs(1),
                    &Cancellation::never()
                )
                .await
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::BrokenPipe
        );
        tokio.shutdown().await;
    });
}

#[test]
fn native_local_capacity_and_drop() {
    let scheduler = Arc::new(
        core_scheduler_native::NativeScheduler::with_options(
            core_scheduler_native::NativeOptions {
                workers: 1,
                timer_capacity: 1,
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let instance = scheduler.clone();
    scheduler
        .block_on(
            scheduler
                .spawn(async move {
                    let never = Cancellation::never();
                    let far = instance.now() + Duration::from_secs(30);
                    let capacity = core_scheduler_timers::local::TimerQueue::<std::task::Waker>::with_capacity(1).capacity();
                    let mut first: Vec<_> = (0..capacity).map(|_| core_scheduler_native::NativeTimer::default()).collect();
                    let mut second = core_scheduler_native::NativeTimer::default();
                    for timer in &mut first { pending_once(instance.timer_wait(timer, far, &never)).await; }
                    assert_eq!(
                        instance
                            .timer_wait(&mut second, far, &never)
                            .await
                            .unwrap_err()
                            .kind(),
                        std::io::ErrorKind::WouldBlock
                    );
                    drop(first);
                    pending_once(instance.timer_wait(&mut second, far, &never)).await;
                })
                .unwrap(),
        )
        .unwrap();
    scheduler.block_on(scheduler.shutdown());
}
