from pathlib import Path
p=Path('/home/samuel/Developer/core-rs/core-scheduler-perf-native-next/crates/iouring/src/io_uring_selector/tests.rs')
s=p.read_text();start=s.index('fn movable_driver_never_upgrades_');end=s.index('\n#[test]',start)
t=s[start:end].replace('''    let (mut driver, (result, buffer)) = std::thread::scope(|scope| {
''','''    let selector_ref = &selector;
    let (mut driver, (result, buffer)) = std::thread::scope(|scope| {
''').replace('''                let output = drive(&mut driver, reading);
                (driver, output)''','''                let output = drive(&mut driver, reading);
                // Keep the issuing thread alive through retirement: thread exit can independently cancel that thread's pending kernel requests.
                drain(selector_ref, &mut driver);
                (driver, output)''').replace('''    block_on(selector.file_close(file)).unwrap();
    drain(&selector, &mut driver);''','''    drop(file);
    assert!(driver.turn(None).unwrap());
    block_on(selector.shutdown());''')
s=s[:start]+t+s[end:];p.write_text(s)
