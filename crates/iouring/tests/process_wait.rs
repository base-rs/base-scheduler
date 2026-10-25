// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#![cfg(target_os = "linux")]

use core_scheduler_iouring::{IoUringOptions, IoUringSelector};

fn run(test: impl AsyncFnOnce(&IoUringSelector)) {
    let (scheduler, driver) = IoUringSelector::new(IoUringOptions::default()).unwrap();
    let thread = std::thread::spawn(move || driver.run());
    futures_lite::future::block_on(test(&scheduler));
    futures_lite::future::block_on(scheduler.shutdown());
    thread.join().unwrap().unwrap();
}
core_scheduler_conformance::process_wait_tests!(run: run);

#[test]
fn pending_shutdown() {
    run(async |scheduler| {
        core_scheduler_conformance::process_wait::shutdown(
            scheduler,
            || scheduler.close(),
            scheduler.shutdown(),
        )
        .await;
    });
}
#[test]
fn dropped_and_forgotten_waits() {
    run(async |scheduler| {
        core_scheduler_conformance::process_wait::abandonment(scheduler, scheduler.shutdown())
            .await;
    });
}

#[cfg(unix)]
#[test]
fn suspended_wait_after_shutdown() {
    run(async |scheduler| {
        core_scheduler_conformance::process_wait::suspended_shutdown(
            scheduler,
            scheduler.shutdown(),
        )
        .await;
    });
}
