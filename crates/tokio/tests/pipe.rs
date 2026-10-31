// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#![cfg(unix)]

use base_scheduler_tokio::TokioScheduler;

fn run(test: impl AsyncFnOnce(&TokioScheduler) + Send + 'static) {
    let (done, completion) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                let scheduler = TokioScheduler::new(runtime.handle().clone());
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    runtime.block_on(test(&scheduler))
                }));
                runtime.block_on(scheduler.shutdown());

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
