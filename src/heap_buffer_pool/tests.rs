// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use futures_lite::future::{block_on, poll_once};
use std::sync::Barrier;

#[test]
fn closing_retires_idle_tokens_across_availability_words() {
    for capacity in [1, WIDTH, WIDTH + 1, WIDTH * 2 + 3] {
        let pool = HeapBufferPool::new(capacity, 8).unwrap();
        let slab = Arc::downgrade(&pool.0.slab);
        assert_eq!(slab.strong_count(), capacity + 1);
        drop(pool);
        assert!(slab.upgrade().is_none());
    }
}

#[test]
fn slot_tokens_survive_pool_close_and_all_frozen_views() {
    let pool = HeapBufferPool::new(3, 8).unwrap();
    let slab = Arc::downgrade(&pool.0.slab);
    let first = pool.try_acquire().unwrap();
    let frozen = pool.try_acquire().unwrap().freeze();
    let view = frozen.view(2..6);
    assert_eq!(slab.strong_count(), 4);
    drop(pool);
    assert_eq!(slab.strong_count(), 2);
    assert_eq!(first.as_ref(), [0; 8]);
    assert_eq!(view.as_ref(), [0; 4]);
    drop(first);
    drop(frozen);
    assert_eq!(slab.strong_count(), 1);
    std::thread::spawn(move || drop(view)).join().unwrap();
    assert!(slab.upgrade().is_none());
}

#[test]
fn concurrent_returns_and_pool_close_reclaim_every_token() {
    for _ in 0..if cfg!(miri) { 4 } else { 300 } {
        let pool = HeapBufferPool::new(8, 8).unwrap();
        let slab = Arc::downgrade(&pool.0.slab);
        let mutable = pool.try_acquire().unwrap();
        let frozen = pool.try_acquire().unwrap().freeze();
        let view = frozen.view(..4);
        let barrier = Arc::new(Barrier::new(4));
        std::thread::scope(|scope| {
            for owner in [
                Box::new(move || drop(mutable)) as Box<dyn FnOnce() + Send>,
                Box::new(move || drop(frozen)),
                Box::new(move || drop(view)),
            ] {
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    owner();
                });
            }
            barrier.wait();
            drop(pool);
        });
        assert!(slab.upgrade().is_none());
    }
}

#[test]
fn abandoned_exhaustion_restores_the_fast_return_path() {
    let pool = HeapBufferPool::new(WIDTH + 1, 8).unwrap();
    let buffers: Vec<_> = (0..pool.capacity())
        .map(|_| pool.try_acquire().unwrap())
        .collect();
    let token = Cancellation::never();
    {
        let mut waiting = pin!(pool.acquire(&token));
        assert!(block_on(poll_once(waiting.as_mut())).is_none());
        assert!(
            pool.0
                .slab
                .free
                .iter()
                .all(|word| word.load(Ordering::Relaxed) & WAITING != 0)
        );
    }
    assert!(
        pool.0
            .slab
            .free
            .iter()
            .all(|word| word.load(Ordering::Relaxed) & WAITING == 0)
    );
    let slab = Arc::downgrade(&pool.0.slab);
    drop(buffers);
    assert_eq!(slab.strong_count(), pool.capacity() + 1);
    drop(pool);
    assert!(slab.upgrade().is_none());
}

#[test]
fn notification_can_synchronously_acquire_and_retain_a_returned_slot() {
    use std::task::{Context, Wake, Waker};

    struct AcquiringWaiter {
        pool: HeapBufferPool,
        acquired: Mutex<Option<BufferLease>>,
    }
    impl Wake for AcquiringWaiter {
        fn wake(self: Arc<Self>) {
            self.wake_by_ref();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            *self.acquired.lock().unwrap() = self.pool.try_acquire();
        }
    }

    let pool = HeapBufferPool::new(1, 8).unwrap();
    let lease = pool.try_acquire().unwrap();
    let callback = Arc::new(AcquiringWaiter {
        pool: pool.clone(),
        acquired: Mutex::new(None),
    });
    let waker = Waker::from(callback.clone());
    let never = Cancellation::never();
    let mut waiting = Box::pin(pool.acquire(&never));
    assert!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    drop(lease);
    assert!(pool.try_acquire().is_none());
    let returned = callback.acquired.lock().unwrap().take().unwrap();
    assert_eq!(returned.as_ref(), [0; 8]);
    drop(waiting);
    drop(returned);
    assert!(pool.try_acquire().is_some());
}
