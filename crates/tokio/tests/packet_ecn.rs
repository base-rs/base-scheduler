// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

// The packet ECN conformance suite over loopback, for every family.

use base_scheduler_conformance::{EcnFamily, LoopbackEcnFixture};
use base_scheduler_tokio::TokioScheduler;
use futures_lite::future::block_on;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::mpsc,
    time::Duration,
};

fn deadline(test: impl FnOnce() + Send + 'static) {
    let (done, completion) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(catch_unwind(AssertUnwindSafe(test)));
    });
    if let Err(payload) = completion
        .recv_timeout(Duration::from_secs(20))
        .expect("packet ECN test deadline exceeded")
    {
        std::panic::resume_unwind(payload);
    }
}

mod multi_thread {
    use super::*;
    macro_rules! family {
        ($module:ident, $family:ident, $suite:ident) => {
            mod $module {
                use super::*;
                fn run(
                    test: impl AsyncFnOnce(&LoopbackEcnFixture<'_, TokioScheduler>) + Send + 'static,
                ) {
                    super::run(EcnFamily::$family, test);
                }
                base_scheduler_conformance::$suite!(run: run);
            }
        };
    }
    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    family!(ipv4, Ipv4, packet_ecn_tests);
    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    family!(ipv6, Ipv6, packet_ecn_tests);
    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    family!(dual_stack, DualStack, packet_ecn_tests);
    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
    family!(unsupported_ipv4, Ipv4, packet_ecn_unsupported_tests);
    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
    family!(unsupported_ipv6, Ipv6, packet_ecn_unsupported_tests);

    fn run(
        family: EcnFamily,
        test: impl AsyncFnOnce(&LoopbackEcnFixture<'_, TokioScheduler>) + Send + 'static,
    ) {
        deadline(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap();
            let scheduler = TokioScheduler::new(runtime.handle().clone());
            let fixture = block_on(LoopbackEcnFixture::open(&scheduler, family));
            let result = catch_unwind(AssertUnwindSafe(|| block_on(test(&fixture))));
            block_on(fixture.close());
            block_on(scheduler.shutdown());
            if let Err(payload) = result {
                std::panic::resume_unwind(payload);
            }
        });
    }
}
