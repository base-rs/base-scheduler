// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

#[test]
fn never_signals_need_no_cancellation_registration() {
    let never = Cancellation::never();
    assert!(never.is_never());
    assert!(never.clone().is_never());
    assert!(!never.child().is_never());
    let token = Cancellation::new();
    assert!(!token.is_never());
    token.cancel();
    assert!(!token.is_never());
}

#[test]
fn dropped_children_release_their_registrations_and_ancestors() {
    let root = Cancellation::new();
    let node = root.node.as_ref().unwrap();
    for _ in 0..100 {
        let child = root.child();
        let grandchild = child.child();
        drop(child);
        drop(grandchild);
        assert!(node.children.lock().unwrap().is_empty());
        assert_eq!(Arc::strong_count(node), 1);
    }
    // Churn reuses one slot rather than growing the registry.
    assert_eq!(node.children.lock().unwrap().slots.len(), 1);
}

#[test]
fn an_exclusively_owned_deep_chain_is_released_without_recursive_destruction() {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| {
            let root = Cancellation::new();
            let weak = Arc::downgrade(root.node.as_ref().unwrap());
            let mut leaf = root.clone();
            for _ in 0..10_000 {
                leaf = leaf.child();
            }
            drop(root);
            assert!(weak.upgrade().is_some());
            drop(leaf);
            assert!(weak.upgrade().is_none());
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn released_child_slots_are_reused_and_still_receive_cancellation() {
    let root = Cancellation::new();
    let node = root.node.as_ref().unwrap();
    let first = root.child();
    let second = root.child();
    let third = root.child();
    drop(second);
    let reused = root.child();
    assert_eq!(node.children.lock().unwrap().slots.len(), 3);
    drop(first);
    {
        let children = node.children.lock().unwrap();
        assert!(children.slots[0].is_none() && children.slots[1].is_some());
    }
    assert!(root.cancel());
    assert!(third.is_cancelled() && reused.is_cancelled());
    assert!(node.children.lock().unwrap().is_empty());
    let late = root.child();
    assert!(late.is_cancelled());
    drop((third, reused, late));
    assert!(node.children.lock().unwrap().slots.is_empty());
}

mod watch {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::task::Wake;

    #[derive(Default)]
    struct Count(AtomicUsize);
    impl Wake for Count {
        fn wake(self: Arc<Self>) {
            self.wake_by_ref();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn waker() -> (Arc<Count>, Waker) {
        let count = Arc::new(Count::default());
        (Arc::clone(&count), Waker::from(count))
    }

    fn poll(cancellation: &Cancellation, waker: &Waker) -> Option<Poll<()>> {
        cancellation.poll_watched(&mut Context::from_waker(waker))
    }

    #[test]
    fn the_owner_registers_once_and_is_woken_once() {
        let root = Cancellation::new();
        let watched = root.watch();
        let (count, waker) = waker();
        assert_eq!(poll(&watched, &waker), Some(Poll::Pending));
        // One clone held by the watch; repeated waits clone nothing more.
        let held = Arc::strong_count(&count);
        for _ in 0..16 {
            assert_eq!(poll(&watched, &waker), Some(Poll::Pending));
        }
        assert_eq!(Arc::strong_count(&count), held);
        assert!(watched.watch.as_ref().unwrap().wakers().others.is_empty());
        assert!(root.cancel());
        assert_eq!(count.0.load(Ordering::SeqCst), 1);
        assert_eq!(poll(&watched, &waker), Some(Poll::Ready(())));
        // Cancelling released the watch's waker.
        assert_eq!(Arc::strong_count(&count), held - 1);
    }

    #[test]
    fn waits_by_other_tasks_are_woken_too() {
        let root = Cancellation::new();
        let watched = root.watch();
        let (owner, owner_waker) = waker();
        let (other, other_waker) = waker();
        assert_eq!(poll(&watched, &owner_waker), Some(Poll::Pending));
        for _ in 0..4 {
            assert_eq!(poll(&watched, &other_waker), Some(Poll::Pending));
        }
        // Another task's waker is registered once, however often it waits.
        assert_eq!(watched.watch.as_ref().unwrap().wakers().others.len(), 1);
        assert!(root.cancel());
        assert_eq!(owner.0.load(Ordering::SeqCst), 1);
        assert_eq!(other.0.load(Ordering::SeqCst), 1);
        assert_eq!(poll(&watched, &other_waker), Some(Poll::Ready(())));
    }

    #[test]
    fn a_watch_observes_cancellation_before_and_after_registering() {
        let early = Cancellation::new();
        let watched = early.watch();
        early.cancel();
        let (count, waker) = waker();
        assert_eq!(poll(&watched, &waker), Some(Poll::Ready(())));
        assert_eq!(count.0.load(Ordering::SeqCst), 0);
        // A watch made after cancellation is ready at once.
        assert_eq!(poll(&early.watch(), &waker), Some(Poll::Ready(())));
    }

    #[test]
    fn an_ancestor_wakes_a_child_watch() {
        let root = Cancellation::new();
        let watched = root.child().watch();
        let (count, waker) = waker();
        assert_eq!(poll(&watched, &waker), Some(Poll::Pending));
        root.cancel();
        assert_eq!(count.0.load(Ordering::SeqCst), 1);
        assert_eq!(poll(&watched, &waker), Some(Poll::Ready(())));
    }

    #[test]
    fn clones_children_and_plain_signals_are_not_watched() {
        let root = Cancellation::new();
        let watched = root.watch();
        assert!(watched.is_watched());
        assert!(!watched.clone().is_watched());
        assert!(!watched.child().is_watched());
        assert!(!root.is_watched());
        let (_, waker) = waker();
        assert_eq!(poll(&root, &waker), None);
        // A plain clone still shares the signal.
        root.cancel();
        assert!(watched.clone().is_cancelled());
    }

    #[test]
    fn a_watch_of_never_is_never() {
        let watched = Cancellation::never().watch();
        assert!(watched.is_never() && !watched.is_watched());
        let (_, waker) = waker();
        assert_eq!(poll(&watched, &waker), Some(Poll::Pending));
    }

    #[test]
    fn dropped_watches_release_their_slots_for_reuse() {
        let root = Cancellation::new();
        let node = Arc::clone(root.node.as_ref().unwrap());
        let first = root.watch();
        let second = root.watch();
        drop(first);
        let reused = root.watch();
        assert_eq!(node.watches.lock().unwrap().slots.len(), 2);
        drop((second, reused));
        assert!(
            node.watches
                .lock()
                .unwrap()
                .slots
                .iter()
                .all(Option::is_none)
        );
        // Dropping after cancellation releases its slot as well.
        let late = root.watch();
        root.cancel();
        drop(late);
        assert!(
            node.watches
                .lock()
                .unwrap()
                .slots
                .iter()
                .all(Option::is_none)
        );
    }

    #[test]
    fn a_watched_wait_through_cancelled_registers_nothing_of_its_own() {
        let root = Cancellation::new();
        let watched = root.watch();
        let (count, waker) = waker();
        let mut context = Context::from_waker(&waker);
        let mut first = std::pin::pin!(watched.cancelled());
        let mut second = std::pin::pin!(watched.cancelled());
        assert!(first.as_mut().poll(&mut context).is_pending());
        assert!(second.as_mut().poll(&mut context).is_pending());
        assert!(watched.watch.as_ref().unwrap().owner.get().is_some());
        root.cancel();
        // Both waits share one registration, woken once.
        assert_eq!(count.0.load(Ordering::SeqCst), 1);
        assert!(first.as_mut().poll(&mut context).is_ready());
        assert!(second.as_mut().poll(&mut context).is_ready());
    }

    #[test]
    fn cancellation_racing_registration_is_never_lost() {
        for _ in 0..if cfg!(miri) { 20 } else { 2_000 } {
            let root = Cancellation::new();
            let watched = root.watch();
            let (count, waker) = waker();
            let cancelling = {
                let root = root.clone();
                std::thread::spawn(move || root.cancel())
            };
            let polled = poll(&watched, &waker);
            cancelling.join().unwrap();
            // Either the poll saw the cancellation or the cancellation woke it.
            assert!(polled == Some(Poll::Ready(())) || count.0.load(Ordering::SeqCst) == 1);
        }
    }
}
