#!/usr/bin/env python3
"""Apply the probe instrumentation to a base-scheduler tree: instrument.py TREE old|new"""
import sys, shutil
from pathlib import Path

tree, kind = Path(sys.argv[1]), sys.argv[2]
native = tree / "crates/native/src"
shutil.copy(Path(__file__).parent / "probe.rs", native / "probe.rs")

def sub(path, old, new, count=1):
    text = path.read_text()
    assert text.count(old) == count, (path, old, text.count(old))
    path.write_text(text.replace(old, new))

sub(native / "lib.rs", "mod worker;\n", "mod worker;\npub mod probe;\n")
# Idle wake: after the idle path's drive returns.
sub(native / "worker.rs", """                        drive(
                            &mut driver,
                            crate::worker_timers::timeout(&shared, identifier),
                        )
""", """                        drive(
                            &mut driver,
                            crate::worker_timers::timeout(&shared, identifier),
                        );
                        crate::probe::woke();
""")
timers = native / "worker_timers.rs"
if kind == "old":
    sub(timers, """fn expire(shared: &Arc<Shared>, owner: usize) {
""", """fn expire(shared: &Arc<Shared>, owner: usize) {
    crate::probe::start_pass();
    let mut fired = 0;
""")
    sub(timers, """    let now = now(shared);
    loop {""", """    let now = now(shared);
    crate::probe::collect(now);
    loop {""")
    sub(timers, """        let Some(wake) = wake else { break };
""", """        let Some(wake) = wake else { break };
        fired += 1;
""")
    # Count at the end of the pass.
    sub(timers, """        crate::panic::contain(|| wake.wake());
    }
}""", """        crate::panic::contain(|| wake.wake());
    }
    crate::probe::fired(fired);
    crate::probe::fired_by(owner, fired);
}""")
else:
    sub(timers, """    let worker = &shared.workers[owner];
    let mut fired = 0;
    loop {""", """    let worker = &shared.workers[owner];
    let mut fired = 0;
    if !rescue { crate::probe::start_pass(); }
    loop {""")
    sub(timers, """        if let Some(now) = now {
            queue.pop_expired_batch(now, expired);""", """        if let Some(now) = now {
            if !rescue { crate::probe::collect(now); }
            queue.pop_expired_batch(now, expired);""")
    sub(timers, """        if !full {
            return fired;
        }""", """        if !full {
            if !rescue { crate::probe::fired(fired); crate::probe::fired_by(owner, fired); }
            return fired;
        }""")
