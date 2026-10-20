from pathlib import Path
p=Path('/home/samuel/Developer/core-rs/core-scheduler-perf-native-next/crates/native/src/worker.rs')
s=p.read_text().replace('''    // Publication precedes the sleeping flag exchange. A worker publishes its sleeping flag before rechecking queues, so a racing wake is retained.
    shared.wake_idle(preferred);''','''    // A single worker has no peer discovery to perform. Keep its sleeping exchange and driver notification even when the wake came from its own completion processing.
    if shared.workers.len() == 1 {
        shared.workers[0].wake();
    } else {
        // Publication precedes the sleeping flag exchange. A worker publishes its sleeping flag before rechecking queues, so a racing wake is retained.
        shared.wake_idle(preferred);
    }''').replace('''            if !current
''','''            if shared.workers.len() > 1 && !current
''')
p.write_text(s)
