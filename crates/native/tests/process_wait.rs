// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler_native::NativeScheduler;

fn run(test: impl AsyncFnOnce(&NativeScheduler)) {
    let scheduler = NativeScheduler::with_options(core_scheduler_native::NativeOptions {
        workers: 1,
        ..Default::default()
    })
    .unwrap();
    scheduler.block_on(test(&scheduler));
    futures_lite::future::block_on(scheduler.shutdown());
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
