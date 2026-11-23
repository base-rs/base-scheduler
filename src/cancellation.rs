// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::Cancelled;
use event_listener::Event;
use std::future::{pending, poll_fn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::task::{Context, Poll, Waker};

struct Node {
    cancelled: AtomicBool,
    event: Event,
    // Serialize child registration and cancellation across this signal family.
    tree: Arc<Mutex<()>>,
    // Children retain ancestors, but ancestors only hold weak child references.
    parent: Option<Arc<Node>>,
    // This node's slot in its parent's children.
    index: usize,
    children: Mutex<Children>,
    // The watches made from this node's signals, which cancelling wakes.
    watches: Mutex<Slots<Arc<Watch>>>,
}

// Weak child registrations, addressed by a slot each child records.
#[derive(Default)]
struct Children {
    slots: Vec<Option<Weak<Node>>>,
    free: Vec<usize>,
}

impl Children {
    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.slots.iter().all(Option::is_none)
    }
    fn next(&self) -> usize {
        self.free.last().copied().unwrap_or(self.slots.len())
    }
    fn insert(&mut self, child: Weak<Node>) {
        match self.free.pop() {
            Some(index) => self.slots[index] = Some(child),
            None => self.slots.push(Some(child)),
        }
    }
    fn remove(&mut self, index: usize) {
        // A live child keeps its slot until it drops, and a cancelled parent
        // takes every slot and registers no further children.
        if let Some(slot) = self.slots.get_mut(index) {
            *slot = None;
            self.free.push(index);
        }
    }
}

impl Node {
    fn new(tree: Arc<Mutex<()>>, parent: Option<Arc<Node>>, index: usize) -> Self {
        let event = Event::new();
        // Allocate shared event state during setup, rather than on the first wait.
        event.notify(0);
        Self {
            cancelled: AtomicBool::new(false),
            event,
            tree,
            parent,
            index,
            children: Mutex::new(Children::default()),
            watches: Mutex::new(Slots::default()),
        }
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        let mut index = self.index;
        let mut parent = self.parent.take();
        // Release an exclusively owned ancestor chain without recursive drops.
        while let Some(node) = parent {
            // A slot is reused only after its child's destruction removes it, so the index cannot alias a later registration.
            node.children
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(index);
            index = node.index;
            match Arc::try_unwrap(node) {
                Ok(mut node) => {
                    parent = node.parent.take();
                }
                Err(_) => break,
            }
        }
    }
}

/// A runtime-independent, cooperative cancellation signal.
///
/// Clones share a persistent cancellation state. Child signals receive ancestor cancellation, but cancelling a child does not cancel its parent or siblings. Dropping a signal does not request cancellation. Signals neither own tasks nor abort futures; callers decide how to respond and separately await completion.
///
/// ```
/// use base_scheduler::{Cancellation, Cancelled};
///
/// let system = Cancellation::new();
/// let server = system.child();
/// assert!(server.cancel());
/// assert_eq!(server.check(), Err(Cancelled));
/// assert!(!system.is_cancelled());
/// assert!(system.cancel());
/// ```
///
/// A task that waits on one signal many times, such as a connection whose reads,
/// writes and handler waits all share its server's cancellation, can [`watch`](Self::watch)
/// it once, so that its waits share one registration rather than each registering.
pub struct Cancellation {
    node: Option<Arc<Node>>,
    // The registration this value's waits share, for a value made by `watch`.
    watch: Option<Arc<Watch>>,
}

/// Clones share the signal, but not a watch: a clone of a watched signal is plain.
impl Clone for Cancellation {
    fn clone(&self) -> Self {
        Self {
            node: self.node.clone(),
            watch: None,
        }
    }
}

impl Drop for Cancellation {
    fn drop(&mut self) {
        if let (Some(node), Some(watch)) = (&self.node, &self.watch) {
            node.watches
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(watch.slot);
        }
    }
}

impl Cancellation {
    /// Create an independent, uncancelled signal.
    pub fn new() -> Self {
        Self {
            node: Some(Arc::new(Node::new(Arc::new(Mutex::new(())), None, 0))),
            watch: None,
        }
    }

    /// Create a signal which cannot be cancelled and requires no allocation. `cancel` always returns false and `cancelled` remains pending forever.
    pub const fn never() -> Self {
        Self {
            node: None,
            watch: None,
        }
    }

    /// Whether this signal can never be cancelled. This is true for [`Self::never`] and its clones; its children are independently cancellable. Backends can omit cancellation registration for these signals.
    pub const fn is_never(&self) -> bool {
        self.node.is_none()
    }

    /// Create a signal cancelled by this signal or any of its ancestors. A child of an already-cancelled signal starts cancelled. A child of a never-cancelled signal is an independent, cancellable signal.
    pub fn child(&self) -> Self {
        let Some(parent) = &self.node else {
            return Self::new();
        };
        let _tree = parent.tree.lock().unwrap_or_else(PoisonError::into_inner);
        let mut children = parent
            .children
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let child = Arc::new(Node::new(
            Arc::clone(&parent.tree),
            Some(Arc::clone(parent)),
            children.next(),
        ));
        if parent.cancelled.load(Ordering::Acquire) {
            child.cancelled.store(true, Ordering::Release);
        } else {
            children.insert(Arc::downgrade(&child));
        }
        Self {
            node: Some(child),
            watch: None,
        }
    }

    /// The same signal, watched: every wait on the returned value shares one
    /// registration with the first task that waits on it, so that task's later
    /// waits take no lock, clone no waker and allocate nothing. Waits by other
    /// tasks are still woken, at the cost of a lock each.
    ///
    /// Call it once in the task that waits, and pass the value by reference to
    /// its operations. Its clones and children are plain, so a watch is never
    /// shared by accident. A watch of [`never`](Self::never) is `never`.
    ///
    /// ```
    /// use base_scheduler::Cancellation;
    ///
    /// let server = Cancellation::new();
    /// // In each connection's task:
    /// let connection = server.child().watch();
    /// assert!(connection.is_watched());
    /// assert!(!connection.clone().is_watched());
    /// ```
    pub fn watch(&self) -> Self {
        let Some(node) = &self.node else {
            return Self::never();
        };
        let mut watches = node.watches.lock().unwrap_or_else(PoisonError::into_inner);
        let mut watch = None;
        watches.insert(|slot| {
            let created = Arc::new(Watch {
                slot,
                owner: OnceLock::new(),
                wakers: Mutex::new(Wakers::default()),
            });
            // Lock once during setup, rather than on the first wait: on some
            // platforms a mutex allocates its storage when first locked.
            drop(created.wakers());
            watch = Some(Arc::clone(&created));
            created
        });
        drop(watches);
        Self {
            node: Some(Arc::clone(node)),
            watch,
        }
    }

    /// Whether this value was made by [`watch`](Self::watch), so its waits share a registration.
    pub fn is_watched(&self) -> bool {
        self.watch.is_some()
    }

    /// Poll for cancellation without a future, for operations that keep their
    /// own state across waits. For a watched signal, this is ready once it is
    /// cancelled, and otherwise ensures that cancelling it wakes `context`'s
    /// waker, through its watch. For [`never`](Self::never) it is pending.
    /// For a plain signal it returns `None`, registering nothing: wait on it
    /// through [`cancelled`](Self::cancelled), whose future keeps the registration.
    pub fn poll_watched(&self, context: &mut Context<'_>) -> Option<Poll<()>> {
        match (&self.node, &self.watch) {
            (None, _) => Some(Poll::Pending),
            (Some(node), Some(watch)) => Some(watch.poll(node, context)),
            (Some(_), None) => None,
        }
    }

    /// Request cancellation of this signal and all its descendants. Returns true only for the first request on this signal. Repeated requests never escalate. Waiters are woken after the entire family lock is released. Concurrent observers may see propagation in progress, but all descendants are cancelled when this call returns. This does not wait for their work.
    pub fn cancel(&self) -> bool {
        let Some(root) = &self.node else {
            return false;
        };
        let tree = root.tree.lock().unwrap_or_else(PoisonError::into_inner);
        if root.cancelled.load(Ordering::Acquire) {
            return false;
        }
        let mut pending = vec![Arc::clone(root)];
        let mut cancelled = Vec::new();
        while let Some(node) = pending.pop() {
            node.cancelled.store(true, Ordering::Release);
            pending.extend(
                std::mem::take(&mut *node.children.lock().unwrap_or_else(PoisonError::into_inner))
                    .slots
                    .into_iter()
                    .flatten()
                    .filter_map(|child| child.upgrade()),
            );
            cancelled.push(node);
        }
        drop(tree);
        for node in cancelled {
            node.event.notify(usize::MAX);
            let wakers: Vec<Waker> = node
                .watches
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .slots
                .iter()
                .flatten()
                .flat_map(|watch| watch.take())
                .collect();
            wakers.into_iter().for_each(Waker::wake);
        }
        true
    }

    /// Whether cancellation has been requested.
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.node
            .as_ref()
            .is_some_and(|node| node.cancelled.load(Ordering::Acquire))
    }

    /// Check for cancellation at an explicit cooperative cancellation point.
    #[inline]
    pub fn check(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }

    /// Wait for cancellation using standard future polling and wakeups. This future is immediately ready if already cancelled. Dropping the wait unregisters its listener without cancelling the signal or other waiters. Listener storage is retained inline in the future, without a separate allocation. A watched signal's wait uses its watch instead, registering nothing of its own.
    pub async fn cancelled(&self) {
        let Some(node) = &self.node else {
            return pending().await;
        };
        if let Some(watch) = &self.watch {
            return poll_fn(|context| watch.poll(node, context)).await;
        }
        event_listener::listener!(node.event => listener);
        // Register before checking to preserve a racing cancellation notification.
        if !node.cancelled.load(Ordering::Acquire) {
            listener.await;
        }
    }
}

// Values addressed by the slot each records, with released slots reused.
struct Slots<T> {
    slots: Vec<Option<T>>,
    free: Vec<usize>,
}

impl<T> Default for Slots<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }
}

impl<T> Slots<T> {
    fn insert(&mut self, value: impl FnOnce(usize) -> T) -> usize {
        match self.free.pop() {
            Some(index) => {
                self.slots[index] = Some(value(index));
                index
            }
            None => {
                let index = self.slots.len();
                self.slots.push(Some(value(index)));
                index
            }
        }
    }

    fn remove(&mut self, index: usize) {
        self.slots[index] = None;
        self.free.push(index);
    }
}

// One registration shared by every wait on a watched signal. The first waker to
// wait is its owner's: a wait whose waker will wake the owner's takes no lock.
// A wait with another waker, as from another task, is added to the others,
// under the lock, so that it is still woken.
struct Watch {
    slot: usize,
    // The owner's waker data and vtable addresses, compared as `Waker::will_wake` does.
    owner: OnceLock<(usize, usize)>,
    wakers: Mutex<Wakers>,
}

#[derive(Default)]
struct Wakers {
    owner: Option<Waker>,
    // Each with the identity of the waker it was cloned from, compared as the owner's is.
    others: Vec<((usize, usize), Waker)>,
}

fn identity(waker: &Waker) -> (usize, usize) {
    (
        waker.data() as usize,
        std::ptr::from_ref(waker.vtable()) as usize,
    )
}

impl Watch {
    fn wakers(&self) -> MutexGuard<'_, Wakers> {
        self.wakers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn poll(&self, node: &Node, context: &mut Context<'_>) -> Poll<()> {
        let waker = context.waker();
        let identity = identity(waker);
        // The owner's waits register nothing more.
        if self.owner.get() != Some(&identity) {
            let mut wakers = self.wakers();
            let owner = *self.owner.get_or_init(|| {
                wakers.owner = Some(waker.clone());
                identity
            });
            if owner != identity && !wakers.others.iter().any(|(other, _)| *other == identity) {
                wakers.others.push((identity, waker.clone()));
            }
        }
        // Checked after registering. Cancelling sets the flag before it takes
        // the registered wakers, under their lock, to wake them: either it finds
        // this waker, or this observes the flag. The owner is woken at most once,
        // and its next poll observes the flag.
        if node.cancelled.load(Ordering::Acquire) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }

    fn take(&self) -> impl Iterator<Item = Waker> {
        let mut wakers = self.wakers();
        let owner = wakers.owner.take();
        let others = std::mem::take(&mut wakers.others);
        owner
            .into_iter()
            .chain(others.into_iter().map(|(_, waker)| waker))
    }
}

impl Default for Cancellation {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
