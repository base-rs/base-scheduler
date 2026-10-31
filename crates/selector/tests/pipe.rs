// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#![cfg(unix)]

use base_scheduler_selector::ReadinessSelector;

fn run(test: impl AsyncFnOnce(&ReadinessSelector) + Send + 'static) {
    let (done, completion) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || {
                let scheduler = ReadinessSelector::new().unwrap();
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    futures_lite::future::block_on(test(&scheduler))
                }));
                futures_lite::future::block_on(scheduler.shutdown());

                if let Err(payload) = outcome {
                    std::panic::resume_unwind(payload);
                }
            },
        )));
    });
    if let Err(payload) = completion
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("pipe conformance deadline exceeded")
    {
        std::panic::resume_unwind(payload);
    }
}
base_scheduler_conformance::pipe_tests!(run: run);
