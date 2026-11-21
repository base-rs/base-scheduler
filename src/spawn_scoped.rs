// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{SpawnError, TaskError};
use std::{
    future::Future,
    marker::PhantomData,
    mem::ManuallyDrop,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    pin::{Pin, pin},
    sync::{
        Arc, Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
    thread::{self, Thread},
};

/// Run tasks that may borrow from the code which started them, inside a
/// blocking scope, as `std::thread::scope` does for threads.
///
/// [`scope`](Self::scope) runs an asynchronous root on the calling thread and
/// returns only after the root has returned and every task spawned through its
/// [`Scope`] has been destroyed. Those tasks run on the scheduler's workers and
/// may borrow anything that outlives the call, including the scheduler.
///
/// Call `scope` outside the scheduler's tasks and workers. It blocks the calling
/// thread; a backend that detects a call from one of its workers panics, and a
/// call from inside other asynchronous code can deadlock. No asynchronous form
/// exists, because a caller could forget an asynchronous scope's future without
/// running its cleanup, and borrowed data would outlive the tasks using it.
///
/// # Safety
///
/// Implementations must uphold the contract of
/// [`spawn_unchecked`](Self::spawn_unchecked), on which the soundness of every
/// scope relies.
pub unsafe trait SpawnScoped {
    /// The backend's owning handle for one scoped task. Dropping it requests
    /// the task's destruction once any active poll returns; it never detaches.
    /// The task reports its own completion, so the handle is never polled.
    type ScopedHandle: Send;

    /// Register `future` before it becomes runnable and run it on the
    /// scheduler's workers. `future` need not be `'static`.
    ///
    /// The implementation must:
    ///
    /// - Never poll or drop `future`, or any storage holding it, after
    ///   releasing `token`, by dropping it; and drop `token` only once nothing
    ///   the backend owns, such as a queued runnable, a task allocation's
    ///   drop path or the returned handle, can poll or drop `future` again.
    /// - Eventually destroy `future` and release `token`, once the future has
    ///   completed or the returned handle has been dropped, including during
    ///   graceful shutdown or abrupt runtime teardown.
    /// - On failure, drop `future` without polling it and release `token`
    ///   before returning the error.
    /// - Contain a panic raised while destroying `future`, as unscoped tasks do.
    ///
    /// # Safety
    ///
    /// The caller must ensure that everything `future` borrows remains valid
    /// until `token` is released.
    unsafe fn spawn_unchecked<F>(
        &self,
        future: F,
        token: ScopeToken,
    ) -> Result<Self::ScopedHandle, SpawnError>
    where
        F: Future<Output = ()> + Send;

    /// Panic if the calling thread must not block on this scheduler, such as
    /// one of its workers. The default accepts every thread.
    fn check_blocking(&self) {}

    /// Run `root` on the calling thread, then block until every task spawned
    /// through its [`Scope`] has been destroyed, and return `root`'s output.
    ///
    /// If `root` panics, every task is still waited for before the panic is
    /// resumed. If a task panicked and its handle was dropped without observing
    /// the panic, this panics once every task has been destroyed.
    fn scope<'env, R, T>(&'env self, root: R) -> T
    where
        Self: Sized,
        R: for<'scope> AsyncFnOnce(&'scope Scope<'scope, 'env, Self>) -> T,
    {
        self.check_blocking();
        let scope = Scope {
            scheduler: self,
            state: Arc::new(ScopeState::default()),
            scope: PhantomData,
            env: PhantomData,
        };
        let output = catch_unwind(AssertUnwindSafe(|| block_on(root(&scope))));
        scope.state.wait_released();
        match output {
            Err(panic) => resume_unwind(panic),
            Ok(_) if scope.state.unobserved_panic.load(Ordering::Acquire) => {
                panic!("a scoped task panicked and its panic was not observed")
            }
            Ok(output) => output,
        }
    }
}

#[derive(Default)]
struct ScopeState {
    /// Tasks whose tokens have not been released.
    live: Mutex<usize>,
    released: Condvar,
    unobserved_panic: AtomicBool,
}

impl ScopeState {
    fn lock(&self) -> MutexGuard<'_, usize> {
        self.live.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn wait_released(&self) {
        let mut live = self.lock();
        while *live != 0 {
            live = self
                .released
                .wait(live)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// A scoped task's claim on its scope: the scope does not return until every
/// token has been released, by dropping it. A backend holds it for as long as
/// anything it owns can reach the task's future.
pub struct ScopeToken {
    state: Arc<ScopeState>,
}

impl ScopeToken {
    fn new(state: &Arc<ScopeState>) -> Self {
        *state.lock() += 1;
        Self {
            state: state.clone(),
        }
    }
}

impl Drop for ScopeToken {
    fn drop(&mut self) {
        let mut live = self.state.lock();
        *live -= 1;
        if *live == 0 {
            self.state.released.notify_all();
        }
    }
}

/// Spawns tasks into a [`SpawnScoped::scope`]. Tasks may borrow anything that
/// outlives `'env`, and may spawn further tasks through a borrowed `&Scope`.
pub struct Scope<'scope, 'env: 'scope, S: SpawnScoped> {
    scheduler: &'env S,
    state: Arc<ScopeState>,
    // Invariant, as `std::thread::Scope` is, so neither lifetime can be shortened.
    scope: PhantomData<&'scope mut &'scope ()>,
    env: PhantomData<&'env mut &'env ()>,
}

impl<'scope, 'env, S: SpawnScoped> Scope<'scope, 'env, S> {
    /// Register `future` before it becomes runnable and run it on the
    /// scheduler's workers. A closed scheduler returns
    /// `SpawnError::SchedulerClosed`, dropping `future` without polling it.
    pub fn spawn<F>(&'scope self, future: F) -> Result<ScopedTask<'scope, S, F::Output>, SpawnError>
    where
        F: Future + Send + 'scope,
        F::Output: Send + 'scope,
    {
        let slot = Arc::new(Slot {
            state: Mutex::new(SlotState {
                output: None,
                abandoned: false,
                waker: None,
            }),
        });
        let task = Delivered {
            future: ManuallyDrop::new(future),
            completed: false,
            slot: slot.clone(),
            scope: self.state.clone(),
        };
        let token = ScopeToken::new(&self.state);
        // SAFETY: `task` borrows only data that outlives `'scope`. `scope`
        // waits for `token` to be released before returning, which ends
        // `'scope`, and the backend releases it only once nothing can poll or
        // drop `task` again.
        let handle = unsafe { self.scheduler.spawn_unchecked(task, token) }?;
        Ok(ScopedTask {
            _handle: handle,
            slot,
            scope: self.state.clone(),
            finished: false,
            lifetime: PhantomData,
        })
    }
}

struct SlotState<T> {
    output: Option<Result<T, TaskError>>,
    /// The task's handle was dropped without taking the output.
    abandoned: bool,
    waker: Option<Waker>,
}

/// A scoped task's output, delivered through storage the scope owns rather
/// than a runtime's, so it is destroyed before the scope returns.
struct Slot<T> {
    state: Mutex<SlotState<T>>,
}

impl<T> Slot<T> {
    fn lock(&self) -> MutexGuard<'_, SlotState<T>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The future a backend runs for a scoped task: it delivers the task's output
/// or panic to its slot, or `TaskError::Cancelled` if it is destroyed first.
struct Delivered<F: Future> {
    future: ManuallyDrop<F>,
    completed: bool,
    slot: Arc<Slot<F::Output>>,
    scope: Arc<ScopeState>,
}

impl<F: Future> Future for Delivered<F> {
    type Output = ();
    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        // SAFETY: `future` is structurally pinned: nothing moves it out, `Drop`
        // destroys it in place, and `Delivered` is `Unpin` only when `F` is.
        // The other fields are not pinned.
        let this = unsafe { self.get_unchecked_mut() };
        // SAFETY: As above; `future` stays in place until it is destroyed.
        let future = unsafe { Pin::new_unchecked(&mut *this.future) };
        let output = match catch_unwind(AssertUnwindSafe(|| future.poll(context))) {
            Ok(Poll::Pending) => return Poll::Pending,
            Ok(Poll::Ready(output)) => Ok(output),
            Err(panic) => Err(TaskError::Panicked(panic)),
        };
        this.completed = true;
        let mut slot = this.slot.lock();
        if slot.abandoned {
            if output.is_err() {
                this.scope.unobserved_panic.store(true, Ordering::Release);
            }
            // Destroy the output now, before the task's token can be released.
            drop(slot);
            drop(output);
            return Poll::Ready(());
        }
        slot.output = Some(output);
        let waker = slot.waker.take();
        drop(slot);
        if let Some(waker) = waker {
            waker.wake();
        }
        Poll::Ready(())
    }
}

impl<F: Future> Drop for Delivered<F> {
    fn drop(&mut self) {
        // SAFETY: `future` is destroyed in place, exactly once, and not used
        // afterwards.
        let destroyed = catch_unwind(AssertUnwindSafe(|| unsafe {
            ManuallyDrop::drop(&mut self.future)
        }));
        if !self.completed {
            let waker = {
                let mut slot = self.slot.lock();
                if slot.abandoned {
                    None
                } else {
                    slot.output = Some(Err(TaskError::Cancelled));
                    slot.waker.take()
                }
            };
            if let Some(waker) = waker {
                waker.wake();
            }
        }
        // The backend contains destructor panics, as for unscoped tasks.
        if let Err(panic) = destroyed {
            resume_unwind(panic);
        }
    }
}

/// The handle of a task spawned into a [`Scope`]. Awaiting it returns the
/// task's output or `TaskError`; dropping it requests the task's destruction
/// once any active poll returns, and the scope waits for that destruction.
///
/// Forgetting the handle, such as with `std::mem::forget`, leaves the task
/// alive and its scope waiting for it.
#[must_use = "await the task or retain its owning handle"]
pub struct ScopedTask<'scope, S: SpawnScoped, T> {
    slot: Arc<Slot<T>>,
    scope: Arc<ScopeState>,
    finished: bool,
    // Dropped after `Drop` marks the slot abandoned, so that the task's
    // destruction does not report a cancellation.
    _handle: S::ScopedHandle,
    lifetime: PhantomData<&'scope ()>,
}

// The backend handle is owned, never polled, so it is not pinned.
impl<S: SpawnScoped, T> Unpin for ScopedTask<'_, S, T> {}

impl<S: SpawnScoped, T> Future for ScopedTask<'_, S, T> {
    type Output = Result<T, TaskError>;
    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut slot = self.slot.lock();
        match slot.output.take() {
            Some(output) => {
                drop(slot);
                self.finished = true;
                Poll::Ready(output)
            }
            None => {
                slot.waker = Some(context.waker().clone());
                Poll::Pending
            }
        }
    }
}

impl<S: SpawnScoped, T> Drop for ScopedTask<'_, S, T> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let output = {
            let mut slot = self.slot.lock();
            slot.abandoned = true;
            slot.output.take()
        };
        if let Some(Err(TaskError::Panicked(_))) = &output {
            self.scope.unobserved_panic.store(true, Ordering::Release);
        }
        drop(output);
    }
}

/// Wakes the thread blocked in [`block_on`].
struct Unpark(Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// Poll `future` on the calling thread until it completes, parking between
/// polls.
fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(Unpark(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
        thread::park();
    }
}
