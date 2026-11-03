// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

/// A lock for short critical sections that never run user code or block, such as a worker's timer queue. Unlocking is a release store, so an uncontended lock and unlock cost one atomic read-modify-write, against two for `std::sync::Mutex`. It allocates nothing.
pub(crate) struct SpinLock<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// The lock grants exclusive access to the value, which may move between the threads that take it.
unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub(crate) fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    pub(crate) fn try_lock(&self) -> Option<SpinGuard<'_, T>> {
        self.locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| SpinGuard(self))
    }

    pub(crate) fn lock(&self) -> SpinGuard<'_, T> {
        let mut spins = 0u32;
        loop {
            if let Some(guard) = self.try_lock() {
                return guard;
            }
            // A holder never blocks, but it can be preempted: spin briefly, then yield.
            if spins < 64 {
                spins += 1;
                std::hint::spin_loop();
            } else {
                std::thread::yield_now();
            }
        }
    }
}

pub(crate) struct SpinGuard<'a, T>(&'a SpinLock<T>);

impl<T> Deref for SpinGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // The guard holds the lock, so no other reference to the value exists.
        unsafe { &*self.0.value.get() }
    }
}

impl<T> DerefMut for SpinGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // The guard holds the lock, so no other reference to the value exists.
        unsafe { &mut *self.0.value.get() }
    }
}

impl<T> Drop for SpinGuard<'_, T> {
    fn drop(&mut self) {
        self.0.locked.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, thread, time::Duration};

    #[test]
    fn contended_lockers_wait_and_observe_each_others_writes() {
        let lock = Arc::new(SpinLock::new(0usize));
        let guard = lock.lock();
        assert!(lock.try_lock().is_none());
        let contender = {
            let lock = Arc::clone(&lock);
            thread::spawn(move || {
                // Waits past its spinning phase until the holder releases.
                *lock.lock() += 1;
            })
        };
        thread::sleep(Duration::from_millis(20));
        drop(guard);
        contender.join().unwrap();
        assert_eq!(*lock.try_lock().unwrap(), 1);
    }
}
