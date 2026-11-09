// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::test_support::{self, TestSource, step};
use base_scheduler::{Clock, OperationError};

async fn semantics<S: Timers>(scheduler: &S) {
    let never = Cancellation::never();
    let mut timer = scheduler.timer();
    let far = scheduler.now() + Duration::from_secs(30);
    pending_once(scheduler.timer_wait_until(&mut timer, far, &never)).await;
    // Pull a pending timer forward, then reuse it after expiry.
    let early = scheduler.now() + Duration::from_millis(2);
    scheduler
        .timer_wait_until(&mut timer, early, &never)
        .await
        .unwrap();
    assert!(scheduler.now() >= early);
    let later = scheduler.now() + Duration::from_millis(2);
    scheduler
        .timer_wait_until(&mut timer, later, &never)
        .await
        .unwrap();
    assert!(scheduler.now() >= later);
    // Extending a live timer must survive an early backend wake without completing early.
    let first = scheduler.now() + Duration::from_millis(2);
    pending_once(scheduler.timer_wait_until(&mut timer, first, &never)).await;
    let extended = scheduler.now() + Duration::from_millis(4);
    scheduler
        .timer_wait_until(&mut timer, extended, &never)
        .await
        .unwrap();
    assert!(scheduler.now() >= extended);
    // Cancellation retires a previously armed wait, without poisoning the resource.
    {
        let cancellation = Cancellation::new();
        let mut waiting = pin!(scheduler.timer_wait_until(&mut timer, far, &cancellation));
        assert!(
            futures_lite::future::poll_once(waiting.as_mut())
                .await
                .is_none()
        );
        cancellation.cancel();
        assert!(matches!(waiting.await, Err(OperationError::Cancelled(_))));
    }
    let cancelled = Cancellation::new();
    cancelled.cancel();
    assert!(
        scheduler
            .timer_wait_until(&mut scheduler.timer(), scheduler.now(), &cancelled)
            .await
            .is_err()
    );
    scheduler
        .timer_wait_until(&mut timer, scheduler.now(), &never)
        .await
        .unwrap();
    // After must not complete at its old, earlier deadline when extended.
    let mut after = After::new(scheduler.timer());
    let first = scheduler.now() + Duration::from_millis(2);
    pending_once(after.wait(scheduler, first, &never)).await;
    let extended = scheduler.now() + Duration::from_millis(4);
    after.wait(scheduler, extended, &never).await.unwrap();
    assert!(scheduler.now() >= extended);
    pending_once(after.wait(scheduler, far, &never)).await;
    after
        .wait(scheduler, scheduler.now(), &never)
        .await
        .unwrap();
    {
        let cancellation = Cancellation::new();
        let mut waiting = pin!(after.wait(scheduler, far, &cancellation));
        assert!(
            futures_lite::future::poll_once(waiting.as_mut())
                .await
                .is_none()
        );
        cancellation.cancel();
        assert!(waiting.await.is_err());
    }
    let cancelled = Cancellation::new();
    cancelled.cancel();
    assert!(after.wait(scheduler, far, &cancelled).await.is_err());
}

#[test]
fn native_external_and_worker_semantics() {
    test_support::run(|| {
        let scheduler = Arc::new(
            base_scheduler_native::NativeScheduler::with_options(
                base_scheduler_native::NativeOptions {
                    workers: 2,
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        step("checking semantics from the root thread");
        scheduler.block_on(semantics(&*scheduler));
        step("checking semantics from a task");
        let instance = scheduler.clone();
        scheduler
            .block_on(
                scheduler
                    .spawn(async move { semantics(&*instance).await })
                    .unwrap(),
            )
            .unwrap();
        scheduler.block_on(scheduler.shutdown());
    });
}
#[test]
fn tokio_semantics() {
    test_support::run(|| {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let scheduler = base_scheduler_tokio::TokioScheduler::new(runtime.handle().clone());
        runtime.block_on(semantics(&scheduler));
        runtime.block_on(scheduler.shutdown());
    });
}

#[tokio::test(start_paused = true)]
async fn deferred_deadline_and_waker_replacement() {
    let scheduler = base_scheduler_tokio::TokioScheduler::new(tokio::runtime::Handle::current());
    let never = Cancellation::never();
    let mut after = After::new(scheduler.timer());
    let first = scheduler.now() + Duration::from_secs(1);
    pending_once(after.wait(&scheduler, first, &never)).await;
    let final_deadline = first + Duration::from_secs(1);
    tokio::time::advance(Duration::from_secs(1)).await;
    // A past armed deadline must rearm for the latest desired deadline.
    pending_once(after.wait(&scheduler, final_deadline, &never)).await;
    let mut waiting = pin!(after.wait(&scheduler, final_deadline, &never));
    assert!(
        futures_lite::future::poll_once(waiting.as_mut())
            .await
            .is_none()
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    waiting.await.unwrap();
    assert!(scheduler.now() >= final_deadline);
}

async fn incompatible<S: Timers>(first: &S, second: &S) {
    let never = Cancellation::never();
    let mut timer = first.timer();
    // Construction binds the resource before any wait is created or polled.
    let result = second
        .timer_wait_until(&mut timer, second.now(), &never)
        .await;
    assert!(
        matches!(result, Err(OperationError::Io(error)) if error.kind() == std::io::ErrorKind::InvalidInput)
    );
    // An unpolled wait must leave the resource compatible with its constructor.
    drop(second.timer_wait_until(&mut timer, second.now(), &never));
    first
        .timer_wait_until(&mut timer, first.now(), &never)
        .await
        .unwrap();
    let result = second
        .timer_wait_until(&mut timer, second.now(), &never)
        .await;
    assert!(
        matches!(result, Err(OperationError::Io(error)) if error.kind() == std::io::ErrorKind::InvalidInput)
    );
    pending_once(first.timer_wait_until(&mut timer, first.now() + Duration::from_secs(30), &never))
        .await;
    let result = second
        .timer_wait_until(&mut timer, second.now(), &never)
        .await;
    assert!(
        matches!(result, Err(OperationError::Io(error)) if error.kind() == std::io::ErrorKind::InvalidInput)
    );
}
#[test]
fn rejects_incompatible_schedulers() {
    test_support::run(|| {
        let one = base_scheduler_native::NativeScheduler::with_options(
            base_scheduler_native::NativeOptions {
                workers: 1,
                ..Default::default()
            },
        )
        .unwrap();
        let two = base_scheduler_native::NativeScheduler::with_options(
            base_scheduler_native::NativeOptions {
                workers: 1,
                ..Default::default()
            },
        )
        .unwrap();
        step("checking two Native schedulers");
        one.block_on(incompatible(&one, &two));
        one.block_on(one.shutdown());
        two.block_on(two.shutdown());
        let rt1 = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let rt2 = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let one = base_scheduler_tokio::TokioScheduler::new(rt1.handle().clone());
        let two = base_scheduler_tokio::TokioScheduler::new(rt2.handle().clone());
        step("checking two Tokio runtimes");
        rt1.block_on(incompatible(&one, &two));

        step("checking two adapters for one Tokio runtime");
        // Separate adapters for one runtime remain compatible.
        let compatible = base_scheduler_tokio::TokioScheduler::new(rt1.handle().clone());
        rt1.block_on(async {
            let never = Cancellation::never();
            let mut timer = one.timer();
            pending_once(one.timer_wait_until(
                &mut timer,
                one.now() + Duration::from_secs(30),
                &never,
            ))
            .await;
            compatible
                .timer_wait_until(&mut timer, one.now() + Duration::from_millis(2), &never)
                .await
                .unwrap();
        });
    });
}

#[test]
fn native_resource_moves_between_tasks_and_executors() {
    test_support::run(|| {
        let scheduler = Arc::new(
            base_scheduler_native::NativeScheduler::with_options(
                base_scheduler_native::NativeOptions {
                    workers: 2,
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        let instance = scheduler.clone();
        let timer = scheduler
            .block_on(
                scheduler
                    .spawn(async move {
                        let mut timer = instance.timer();
                        let deadline = instance.now() + Duration::from_secs(30);
                        pending_once(instance.timer_wait_until(
                            &mut timer,
                            deadline,
                            &Cancellation::never(),
                        ))
                        .await;
                        (timer, deadline)
                    })
                    .unwrap(),
            )
            .unwrap();
        let instance = scheduler.clone();
        step("polling the timer from another thread");
        // Poll from another executor/thread; update and release still target the original owner.
        std::thread::spawn(move || {
            futures_lite::future::block_on(async move {
                let (mut timer, deadline) = timer;
                let never = Cancellation::never();
                let mut waiting = pin!(instance.timer_wait_until(&mut timer, deadline, &never));
                assert!(
                    futures_lite::future::poll_once(waiting.as_mut())
                        .await
                        .is_none()
                );
            })
        })
        .join()
        .unwrap();
        scheduler.block_on(scheduler.shutdown());
    });
}

#[test]
fn native_capacity_and_resource_drop() {
    test_support::run(|| {
        let scheduler = base_scheduler_native::NativeScheduler::with_options(
            base_scheduler_native::NativeOptions {
                workers: 1,
                timer_capacity: 1,
                ..Default::default()
            },
        )
        .unwrap();
        scheduler.block_on(async {
            let never = Cancellation::never();
            let far = scheduler.now() + Duration::from_secs(30);
            let capacity =
                base_scheduler_timers::local::TimerQueue::<std::task::Waker>::with_capacity(1)
                    .capacity();
            let mut first: Vec<_> = (0..capacity).map(|_| scheduler.timer()).collect();
            // Extra resources do not reserve queue entries until their waits are polled.
            let mut second = scheduler.timer();
            for timer in &mut first {
                pending_once(scheduler.timer_wait_until(timer, far, &never)).await;
            }
            assert_eq!(
                scheduler
                    .timer_wait_until(&mut second, far, &never)
                    .await
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::WouldBlock
            );
            drop(first);
            pending_once(scheduler.timer_wait_until(&mut second, far, &never)).await;
            scheduler.close();
            assert_eq!(
                scheduler
                    .timer_wait_until(&mut second, far, &never)
                    .await
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::BrokenPipe
            );
        });
        scheduler.block_on(scheduler.shutdown());
    });
}

#[test]
fn closed_backends_reject_pending_timers() {
    test_support::run(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let tokio = base_scheduler_tokio::TokioScheduler::new(runtime.handle().clone());
        tokio.close();
        runtime.block_on(async {
            assert_eq!(
                tokio
                    .timer_wait_until(
                        &mut tokio.timer(),
                        tokio.now() + Duration::from_secs(1),
                        &Cancellation::never()
                    )
                    .await
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::BrokenPipe
            );
            tokio.shutdown().await;
        });
    });
}

#[test]
fn native_local_capacity_and_drop() {
    test_support::run(|| {
        let scheduler = Arc::new(
            base_scheduler_native::NativeScheduler::with_options(
                base_scheduler_native::NativeOptions {
                    workers: 1,
                    timer_capacity: 1,
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        let instance = scheduler.clone();
        scheduler
            .block_on(
                scheduler
                    .spawn(async move {
                        let never = Cancellation::never();
                        let far = instance.now() + Duration::from_secs(30);
                        let capacity = base_scheduler_timers::local::TimerQueue::<std::task::Waker>::with_capacity(1).capacity();
                        let mut first: Vec<_> = (0..capacity).map(|_| instance.timer()).collect();
                        let mut second = instance.timer();
                        for timer in &mut first { pending_once(instance.timer_wait_until(timer, far, &never)).await; }
                        assert_eq!(
                            instance
                                .timer_wait_until(&mut second, far, &never)
                                .await
                                .unwrap_err()
                                .kind(),
                            std::io::ErrorKind::WouldBlock
                        );
                        drop(first);
                        pending_once(instance.timer_wait_until(&mut second, far, &never)).await;
                    })
                    .unwrap(),
            )
            .unwrap();
        scheduler.block_on(scheduler.shutdown());
    });
}

#[test]
fn benchmark_counts_immediate_expirations() {
    test_support::run(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let scheduler = base_scheduler_tokio::TokioScheduler::new(runtime.handle().clone());
        for mode in [Mode::Sleep, Mode::Reuse, Mode::After] {
            step(format!("running {mode:?} timers"));
            let (operations, overshoot, expired) = runtime.block_on(workload(
                &scheduler,
                mode,
                TimerWorkload::Cycle,
                2,
                Duration::ZERO,
            ));
            assert_eq!(operations, 66);
            assert_eq!(overshoot.len(), 2);
            assert_eq!(expired, 64);
        }
        runtime.block_on(scheduler.shutdown());
    });
}

fn record(arguments: &[&str]) -> serde_json::Value {
    let benchmark = Benchmark::parse(arguments.iter().map(|value| value.to_string())).unwrap();
    let mut out = Vec::new();
    measure(&benchmark, TestSource::default())
        .unwrap()
        .emit(&mut out)
        .unwrap();
    serde_json::from_slice(&out).unwrap()
}

fn metric<'a>(record: &'a serde_json::Value, name: &str) -> Option<&'a serde_json::Value> {
    record["samples"][0]["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|metric| metric["name"] == name)
}

#[test]
fn every_backend_mode_and_timer_workload_records_its_operations() {
    test_support::run(|| {
        for backend in Backend::ALL {
            for mode in Mode::ALL {
                for workload in TimerWorkload::ALL {
                    let arguments = [
                        "--backend",
                        backend.name(),
                        "--mode",
                        mode.name(),
                        "--workload",
                        workload.name(),
                        "--tasks",
                        "2",
                        "--iterations",
                        "2",
                        "--workers",
                        "1",
                        "--delay-us",
                        "100",
                    ];
                    step(format!("recording {arguments:?}"));
                    let record = record(&arguments);
                    let context = format!("{arguments:?}: {record}");
                    assert_eq!(record["workload"], WORKLOAD);
                    assert_eq!(record["parameters"]["backend"], backend.name(), "{context}");
                    assert_eq!(record["parameters"]["mode"], mode.name(), "{context}");
                    let operations = record["samples"][0]["operations"].as_u64().unwrap();
                    #[cfg(feature = "allocations")]
                    if *workload == TimerWorkload::Alloc {
                        assert_eq!(operations, 2, "{context}");
                        assert!(metric(&record, "allocations").is_some(), "{context}");
                        assert!(metric(&record, "timer_size").is_some(), "{context}");
                        continue;
                    }
                    let per_task = if *workload == TimerWorkload::Cycle {
                        66
                    } else {
                        2
                    };
                    assert_eq!(operations, 2 * per_task, "{context}");
                    assert!(metric(&record, "wall_time").is_some(), "{context}");
                    assert!(
                        metric(&record, "refresh_expirations").is_some(),
                        "{context}"
                    );
                    let expiring = matches!(workload, TimerWorkload::Expire | TimerWorkload::Cycle);
                    assert_eq!(
                        metric(&record, "overshoot_p99").is_some(),
                        expiring,
                        "{context}"
                    );
                    let rescues = *backend != Backend::Tokio && cfg!(feature = "diagnostics");
                    assert_eq!(
                        metric(&record, "rescued_timers").is_some(),
                        rescues,
                        "{context}"
                    );
                }
            }
        }
    });
}

#[test]
fn every_family_measures_into_a_record() {
    test_support::run(|| {
        step("recording the spawn scenario");
        let scenario = record(&["--workload", "spawn", "--iterations", "3"]);
        assert_eq!(scenario["parameters"]["workload"], "spawn");
        assert_eq!(scenario["samples"][0]["operations"], 3);
        step("recording the periodic diagnostic");
        let periodic = record(&[
            "--workload",
            "periodic",
            "--workers",
            "1",
            "--tasks",
            "2",
            "--measure-expiries",
            "2",
        ]);
        assert_eq!(periodic["parameters"]["workload"], "periodic");
        assert_eq!(periodic["samples"][0]["operations"], 2 * 2);
        assert!(metric(&periodic, "overshoot_p50").is_some());
    });
}

#[test]
fn overshoot_reports_order_statistics_of_the_expiries() {
    let sample = with_overshoot(Sample::new(1), Vec::new());
    assert!(sample.metrics.is_empty());
    let sample = with_overshoot(Sample::new(1), (1..=100).rev().collect());
    let values: Vec<_> = sample
        .metrics
        .iter()
        .map(|metric| (metric.name.as_str(), metric.measurement.clone()))
        .collect();
    let quantile = |quantile, value| base_benchmark::Measurement::Quantile { quantile, value };
    assert_eq!(
        values,
        [
            ("overshoot_p50", quantile(0.5, 50.0)),
            ("overshoot_p99", quantile(0.99, 99.0)),
            ("overshoot_max", quantile(1.0, 100.0)),
        ]
    );
}

#[test]
fn commands_write_one_record_or_fail_with_a_message() {
    test_support::run(|| {
        let arguments = |values: &[&str]| {
            values
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        };
        let mut out = Vec::new();
        let mut errors = Vec::new();
        step("running a command that succeeds");
        let status = command_with(
            arguments(&["--iterations", "3"]),
            TestSource::default(),
            &mut out,
            &mut errors,
        );
        assert_eq!(status, ExitCode::SUCCESS);
        assert!(errors.is_empty());
        let record: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(record["samples"][0]["operations"], 3);

        for (values, source, message) in [
            (
                arguments(&["--tasks", "0"]),
                TestSource::default(),
                "--tasks must be positive",
            ),
            (
                arguments(&["--backend", "tokio"]),
                TestSource::failing(),
                "no resource usage",
            ),
            (arguments(&[]), TestSource::failing(), "no resource usage"),
            (
                arguments(&[]),
                TestSource::failing_after(1),
                "no resource usage",
            ),
        ] {
            step(format!(
                "running a command expected to fail with {message:?}"
            ));
            let mut out = Vec::new();
            let mut errors = Vec::new();
            assert_eq!(
                command_with(values, source, &mut out, &mut errors),
                ExitCode::FAILURE
            );
            assert!(out.is_empty());
            let errors = String::from_utf8(errors).unwrap();
            assert!(
                errors.starts_with("base-scheduler-benchmark-timers: "),
                "{errors}"
            );
            assert!(errors.contains(message), "{errors}");
        }
        #[cfg(target_os = "linux")]
        assert_eq!(
            command_with(
                arguments(&["--backend", "iouring"]),
                TestSource::failing(),
                &mut Vec::new(),
                &mut Vec::new()
            ),
            ExitCode::FAILURE
        );
    });
}
