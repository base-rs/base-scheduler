// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::{BufferPool, Cancellation, IoBuf};
use futures_lite::future::{block_on, poll_once};
use std::pin::pin;

#[test]
fn pools_validate_capacity_and_recycle_stable_initialized_storage() {
    for (count, size) in [(0, 1), (1, 0), (usize::MAX, 2)] {
        assert!(BufferPool::new(count, size).is_err());
    }
    let pool = BufferPool::new(1, 16).unwrap();
    assert_eq!(pool.capacity(), 1);
    assert_eq!(pool.buffer_size(), 16);
    let mut buffer = pool.try_acquire().unwrap();
    let pointer = buffer.as_ref().as_ptr();
    buffer.as_mut().fill(42);
    assert!(pool.try_acquire().is_none());
    drop(buffer);
    let buffer = pool.try_acquire().unwrap();
    assert_eq!(pointer, buffer.as_ref().as_ptr());
    assert_eq!(buffer.as_ref(), [42; 16]);
}

#[test]
fn frozen_views_hold_storage_until_the_final_cross_thread_return() {
    let pool = BufferPool::new(1, 16).unwrap();
    let mut buffer = pool.try_acquire().unwrap();
    buffer.as_mut()[..6].copy_from_slice(b"abcdef");
    buffer.truncate(6);
    let pointer = buffer.as_ref().as_ptr() as usize;
    let frozen = buffer.freeze();
    let first = frozen.view(..3);
    let last = frozen.view(3..);
    assert!(format!("{frozen:?}").contains('6'));
    drop(frozen);
    drop(first);
    assert!(pool.try_acquire().is_none());
    std::thread::spawn(move || {
        assert_eq!(last.as_ref(), b"def");
        drop(last);
    })
    .join()
    .unwrap();
    assert_eq!(
        pool.try_acquire().unwrap().as_ref().as_ptr() as usize,
        pointer
    );
}

#[test]
fn leases_outlive_the_pool_handle_and_can_be_used_as_owned_ranges() {
    let pool = BufferPool::new(1, 16).unwrap();
    let buffer = pool.try_acquire().unwrap();
    drop(pool);
    let range = buffer.slice(4..8);
    assert_eq!(range.as_ref(), [0; 4]);
    let buffer = range.into_inner();
    assert!(format!("{buffer:?}").contains("16"));
    let frozen = buffer.freeze();
    assert_eq!(frozen.as_ref(), [0; 16]);
}

#[test]
fn exhaustion_waits_for_capacity_and_cancellation_does_not_take_a_buffer() {
    block_on(async {
        let pool = BufferPool::new(1, 16).unwrap();
        let lease = pool.try_acquire().unwrap();
        let token = Cancellation::new();
        let mut waiting = pin!(pool.acquire(&token));
        assert!(poll_once(waiting.as_mut()).await.is_none());
        token.cancel();
        assert!(waiting.await.is_err());
        assert!(pool.try_acquire().is_none());
        drop(lease);
        assert!(pool.acquire(&token).await.is_err());
        let never = Cancellation::never();
        let lease = pool.acquire(&never).await.unwrap();
        let other = pool.clone();
        let mut waiting = pin!(other.acquire(&never));
        assert!(poll_once(waiting.as_mut()).await.is_none());
        std::thread::spawn(move || drop(lease)).join().unwrap();
        assert_eq!(waiting.await.unwrap().as_ref().len(), 16);
    });
}

#[test]
fn abandoning_waits_and_forgetting_owners_are_safe() {
    let pool = BufferPool::new(1, 16).unwrap();
    let lease = pool.try_acquire().unwrap();
    let token = Cancellation::never();
    {
        let mut wait = pin!(pool.acquire(&token));
        assert!(block_on(poll_once(wait.as_mut())).is_none());
    }
    drop(lease);
    let lease = pool.try_acquire().unwrap();
    std::mem::forget(lease);
    assert!(pool.try_acquire().is_none());
}

#[cfg(not(miri))]
#[test]
fn steady_state_acquire_freeze_views_and_return_allocate_nothing() {
    let pool = BufferPool::new(2, 64).unwrap();
    let info = allocation_counter::measure(|| {
        for _ in 0..128 {
            let mut lease = pool.try_acquire().unwrap();
            lease.as_mut().fill(42);
            lease.truncate(32);
            let frozen = lease.freeze();
            let first = frozen.view(..16);
            let second = frozen.view(16..);
            std::hint::black_box((first.as_ref(), second.as_ref()));
        }
    });
    assert_eq!(info.count_total, 0);
}

#[test]
#[should_panic(expected = "buffer length out of bounds")]
fn leases_cannot_expose_bytes_outside_the_initialized_prefix() {
    let mut lease = BufferPool::new(1, 16).unwrap().try_acquire().unwrap();
    lease.truncate(17);
}

#[test]
fn many_readers_return_each_slot_once() {
    let pool = BufferPool::new(4, 64).unwrap();
    for _ in 0..20 {
        let buffers: Vec<_> = (0..4)
            .map(|_| pool.try_acquire().unwrap().freeze())
            .collect();
        std::thread::scope(|scope| {
            for buffer in &buffers {
                for _ in 0..4 {
                    let view = buffer.clone();
                    scope.spawn(move || assert_eq!(view.as_ref(), [0; 64]));
                }
            }
        });
        assert!(pool.try_acquire().is_none());
        drop(buffers);
    }
    let leases: Vec<_> = (0..4).map(|_| pool.try_acquire().unwrap()).collect();
    assert!(pool.try_acquire().is_none());
    drop(leases);
}
