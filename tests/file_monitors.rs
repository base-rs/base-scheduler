// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::backend::FileMonitor;
use std::sync::Arc;

#[test]
fn backend_registration_mapping_preserves_the_retained_resource() {
    let file = Arc::new(());
    let monitor = FileMonitor::new(Arc::clone(&file), 3);
    let mut monitor = monitor.map_registration(|value| value + 1);
    assert!(Arc::ptr_eq(monitor.file(), &file));
    assert_eq!(Arc::strong_count(&file), 2);
    assert_eq!(monitor.registration(), &4);
    *monitor.registration_mut() = 5;
    assert_eq!(monitor.registration(), &5);
    drop(monitor);
    assert_eq!(Arc::strong_count(&file), 1);
}
