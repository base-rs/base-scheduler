// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

#[test]
fn a_timed_window_ends_at_its_instant_and_counts_requests_completed_by_it() {
    let end = Instant::now();
    let window = End::At(end);
    let before = end - Duration::from_nanos(1);
    let after = end + Duration::from_nanos(1);
    // The number counted never ends a timed window.
    assert!(!window.reached(u64::MAX, before));
    assert!(window.reached(0, end));
    assert!(window.includes(end));
    assert!(!window.includes(after));
}

#[test]
fn a_window_of_requests_ends_at_its_count_and_counts_every_completion() {
    let now = Instant::now();
    let window = End::After(2);
    assert!(!window.reached(1, now + Duration::from_secs(3600)));
    assert!(window.reached(2, now));
    assert!(window.includes(now + Duration::from_secs(3600)));
}
