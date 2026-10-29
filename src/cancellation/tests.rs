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
