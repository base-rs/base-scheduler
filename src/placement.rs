// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{Scope, SocketOperation, SpawnError, SpawnScoped, TaskError};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// Where work runs: polled in place by whoever polls its caller, or as a task
/// of a [`Scope`]. A library that starts independent work, such as a server's
/// connections, takes a placement and writes its supervision once; the caller
/// chooses by passing the scheduler, `&scheduler`, or a scope.
///
/// Work placed by the scheduler is the work's own future, wrapped without
/// allocation, so callers that poll it in place cost what polling the work
/// directly does. Work placed by a scope runs on the scheduler's workers; see
/// [`SpawnScoped::scope`] for what it may borrow.
pub trait Placement<'a>: Copy + Send + Sync {
    /// The scheduler the work uses.
    type Scheduler: ?Sized;

    /// The scheduler, borrowed for as long as placed work may borrow it.
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
// placement serves implements `SocketOperation`, and `Scope` does not.
impl<'a, S: SocketOperation + ?Sized> Placement<'a> for &'a S {
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

impl<'scope, 'env, S: SpawnScoped + Sync> Placement<'scope> for &'scope Scope<'scope, 'env, S> {
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

/// Work polled in place, its output wrapped as a placement's.
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
