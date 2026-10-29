// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::Cancelled;
use event_listener::Event;
use std::future::pending;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

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
#[derive(Clone)]
pub struct Cancellation {
    node: Option<Arc<Node>>,
}

impl Cancellation {
    /// Create an independent, uncancelled signal.
    pub fn new() -> Self {
        Self {
            node: Some(Arc::new(Node::new(Arc::new(Mutex::new(())), None, 0))),
        }
    }

    /// Create a signal which cannot be cancelled and requires no allocation. `cancel` always returns false and `cancelled` remains pending forever.
    pub const fn never() -> Self {
        Self { node: None }
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
        Self { node: Some(child) }
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

    /// Wait for cancellation using standard future polling and wakeups. This future is immediately ready if already cancelled. Dropping the wait unregisters its listener without cancelling the signal or other waiters. Listener storage is retained inline in the future, without a separate allocation.
    pub async fn cancelled(&self) {
        let Some(node) = &self.node else {
            return pending().await;
        };
        event_listener::listener!(node.event => listener);
        // Register before checking to preserve a racing cancellation notification.
        if !node.cancelled.load(Ordering::Acquire) {
            listener.await;
        }
    }
}

impl Default for Cancellation {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
