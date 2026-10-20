from pathlib import Path
p=Path('/home/samuel/Developer/core-rs/core-scheduler-perf-native-next/crates/iouring/src/io_uring_selector/tests.rs')
s=p.read_text()
s+='''

fn drive_bound_worker<F: Future>(
    worker: &mut impl core_scheduler_selector::SelectorWorker,
    future: F,
) -> F::Output {
    let mut future = pin!(future);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(output) = block_on(poll_once(&mut future)) {
            return output;
        }
        worker.turn(Some(Duration::from_millis(10))).unwrap();
        assert!(Instant::now() < deadline, "bound worker did not complete accepted I/O");
    }
}

fn drain_bound_worker(
    selector: &IoUringSelector,
    worker: &mut impl core_scheduler_selector::SelectorWorker,
) {
    worker.close();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !worker.turn(Some(Duration::from_millis(10))).unwrap() {
        assert!(Instant::now() < deadline, "bound worker did not retire accepted I/O");
    }
    block_on(selector.shutdown());
}

#[test]
fn movable_driver_never_upgrades_and_completes_inflight_io_after_thread_migration() {
    let _failures = Failures::new([]);
    BEFORE_SYSCALL.with(|hook| {
        *hook.borrow_mut() = Some(("cooperative-ring", Box::new(|| {
            panic!("a movable driver must preserve cross-thread ring progress");
        })));
    });
    let (selector, mut driver) = pair(1);
    let (directory, file) = fifo(&selector);
    let never = Cancellation::never();
    let mut reading = Box::pin(selector.file_read(&file, vec![0], &never));
    assert!(block_on(poll_once(&mut reading)).is_none());
    driver.turn(Some(Duration::ZERO)).unwrap();
    assert_eq!(driver.reactor.as_ref().unwrap().in_flight.0.len(), 1);
    assert!(block_on(poll_once(&mut reading)).is_none());
    std::fs::write(directory.path().join("fifo"), b"x").unwrap();
    let (mut driver, (result, buffer)) = std::thread::scope(|scope| {
        scope.spawn(move || {
            let output = drive(&mut driver, reading);
            (driver, output)
        }).join().unwrap()
    });
    assert_eq!(result.unwrap(), 1);
    assert_eq!(buffer, b"x");
    block_on(selector.file_close(file)).unwrap();
    drain(&selector, &mut driver);
    BEFORE_SYSCALL.with(|hook| assert!(hook.borrow().is_some()));
}

#[test]
fn fresh_bound_worker_attempts_optional_upgrade_without_losing_queued_io() {
    use core_scheduler_selector::SelectorDriver;
    let (selector, driver) = pair(1);
    let (_directory, file) = file(&selector);
    let never = Cancellation::never();
    let mut reading = Box::pin(selector.file_read_at(&file, vec![0; 3], 1, &never));
    assert!(block_on(poll_once(&mut reading)).is_none());
    let _failures = Failures::new([]);
    let attempted = Arc::new(AtomicBool::new(false));
    BEFORE_SYSCALL.with(|hook| {
        let attempted = Arc::clone(&attempted);
        *hook.borrow_mut() = Some(("cooperative-ring", Box::new(move || {
            attempted.store(true, Ordering::SeqCst);
        })));
    });
    let mut worker = driver.enter();
    assert!(attempted.load(Ordering::SeqCst));
    let (result, buffer) = drive_bound_worker(&mut worker, reading);
    assert_eq!(result.unwrap(), 3);
    assert_eq!(buffer, b"bcd");
    block_on(selector.file_close(file)).unwrap();
    drain_bound_worker(&selector, &mut worker);
}

#[test]
fn optional_worker_upgrade_failures_preserve_the_valid_ring_and_queued_io() {
    use core_scheduler_selector::SelectorDriver;
    for code in [libc::EINVAL, libc::EPERM] {
        let (selector, driver) = pair(1);
        let (_directory, file) = file(&selector);
        let never = Cancellation::never();
        let mut reading = Box::pin(selector.file_read(&file, vec![0; 6], &never));
        assert!(block_on(poll_once(&mut reading)).is_none());
        let _failures = Failures::new([("cooperative-ring", io::Error::from_raw_os_error(code))]);
        let mut worker = driver.enter();
        FAILURES.with(|failures| assert!(failures.borrow().is_empty()));
        let (result, buffer) = drive_bound_worker(&mut worker, reading);
        assert_eq!(result.unwrap(), 6);
        assert_eq!(buffer, b"abcdef");
        block_on(selector.file_close(file)).unwrap();
        drain_bound_worker(&selector, &mut worker);
    }
}

#[test]
fn binding_an_already_driven_ring_preserves_inflight_io_without_upgrade() {
    use core_scheduler_selector::SelectorDriver;
    let (selector, mut driver) = pair(1);
    let (directory, file) = fifo(&selector);
    let never = Cancellation::never();
    let mut reading = Box::pin(selector.file_read(&file, vec![0], &never));
    assert!(block_on(poll_once(&mut reading)).is_none());
    driver.turn(Some(Duration::ZERO)).unwrap();
    assert_eq!(driver.reactor.as_ref().unwrap().in_flight.0.len(), 1);
    assert!(block_on(poll_once(&mut reading)).is_none());
    let _failures = Failures::new([]);
    BEFORE_SYSCALL.with(|hook| {
        *hook.borrow_mut() = Some(("cooperative-ring", Box::new(|| {
            panic!("an already-driven ring must retain its accepted work");
        })));
    });
    let mut worker = driver.enter();
    BEFORE_SYSCALL.with(|hook| assert!(hook.borrow().is_some()));
    std::fs::write(directory.path().join("fifo"), b"x").unwrap();
    let (result, buffer) = drive_bound_worker(&mut worker, reading);
    assert_eq!(result.unwrap(), 1);
    assert_eq!(buffer, b"x");
    block_on(selector.file_close(file)).unwrap();
    drain_bound_worker(&selector, &mut worker);
}
'''
p.write_text(s)
