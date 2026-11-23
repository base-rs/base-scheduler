// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{Scope, SocketOperation, SpawnError, SpawnScoped, TaskError};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// The parent of work that a library starts, such as a server's connections,
/// as Async's `parent:` is the task, barrier or semaphore that runs and owns
/// the tasks it is given. The work cannot outlive its parent. A library takes
/// a parent and writes its supervision once; the caller chooses by passing the
/// scheduler, `&scheduler`, or a [`Scope`]:
///
/// - Given the scheduler, the work is part of the calling future, as Async's
///   default parent is the current task: it is polled in place by whoever
///   polls its caller. Its future is the work's own, wrapped without
///   allocation, so polling it costs what polling the work directly does.
/// - Given a scope, the work runs as a task of the scope, on the scheduler's
///   workers, as a barrier's tasks do; see [`SpawnScoped::scope`] for what it
///   may borrow.
///
/// This trait was named `Placement` in 0.6.2, which remains an alias for it
/// until 0.7.
pub trait Parent<'a>: Copy + Send + Sync {
    /// The scheduler the work uses.
    type Scheduler: ?Sized;

    /// The scheduler, borrowed for as long as the parent's work may borrow it.
    fn scheduler(self) -> &'a Self::Scheduler;

    /// Place `work`, returning a future of its output that owns it: dropping
    /// the future requests the work's destruction, as dropping a task handle
    /// does. Awaiting it returns `Err` only for spawned work, when the task
    /// panicked or was destroyed first. Placing fails only when spawning fails,
    /// as when the scheduler is closed, dropping `work` without polling it.
    fn place<F>(
        self,
        work: F,
    ) -> Result<impl Future<Output = Result<F::Output, TaskError>> + Send + 'a, SpawnError>
    where
        F: Future + Send + 'a,
        F::Output: Send + 'a;
}

// Telling the two implementations apart relies on `Scope` implementing no
// scheduler capability, which only this crate can guarantee: every scheduler a
// parent serves implements `SocketOperation`, and `Scope` does not.
impl<'a, S: SocketOperation + ?Sized> Parent<'a> for &'a S {
    type Scheduler = S;

    fn scheduler(self) -> &'a S {
        self
    }

    fn place<F>(
        self,
        work: F,
    ) -> Result<impl Future<Output = Result<F::Output, TaskError>> + Send + 'a, SpawnError>
    where
        F: Future + Send + 'a,
        F::Output: Send + 'a,
    {
        Ok(InPlace(work))
    }
}

impl<'scope, 'env, S: SpawnScoped + Sync> Parent<'scope> for &'scope Scope<'scope, 'env, S> {
    type Scheduler = S;

    fn scheduler(self) -> &'scope S {
        Scope::scheduler(self)
    }

    fn place<F>(
        self,
        work: F,
    ) -> Result<impl Future<Output = Result<F::Output, TaskError>> + Send + 'scope, SpawnError>
    where
        F: Future + Send + 'scope,
        F::Output: Send + 'scope,
    {
        self.spawn(work)
    }
}

/// Work polled in place, its output wrapped as a parent's.
struct InPlace<F>(F);

impl<F: Future> Future for InPlace<F> {
    type Output = Result<F::Output, TaskError>;
    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: The work is structurally pinned: `InPlace` never moves it,
        // has no `Drop` implementation, and is `Unpin` only when `F` is.
        let work = unsafe { self.map_unchecked_mut(|this| &mut this.0) };
        work.poll(context).map(Ok)
    }
}
