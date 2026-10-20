from pathlib import Path
p=Path('/home/samuel/Developer/core-rs/core-scheduler-perf-native-next/crates/iouring/src/io_uring_selector/tests.rs')
s=p.read_text();start=s.index('fn drain_bound_worker(');end=s.index('\n#[test]',start);s=s[:start]+s[end:]
s=s.replace('drain_bound_worker(&selector, &mut worker);','drive_bound_worker(&mut worker, selector.shutdown());')
s+='''

#[test]
fn a_retired_driver_can_enter_a_worker_without_recreating_its_ring() {
    use core_scheduler_selector::{SelectorDriver, SelectorWorker};
    let (selector, mut driver) = pair(1);
    drain(&selector, &mut driver);
    assert!(driver.reactor.is_none());
    let _failures = Failures::new([]);
    BEFORE_SYSCALL.with(|hook| {
        *hook.borrow_mut() = Some(("cooperative-ring", Box::new(|| {
            panic!("a retired driver must not create another ring");
        })));
    });
    let mut worker = driver.enter();
    assert!(worker.turn(None).unwrap());
    BEFORE_SYSCALL.with(|hook| assert!(hook.borrow().is_some()));
    drop(worker);
    block_on(selector.shutdown());
}
'''
p.write_text(s)
