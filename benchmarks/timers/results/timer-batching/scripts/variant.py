#!/usr/bin/env python3
"""variant.py TREE NAME: apply an experimental expiry variant to a main-based tree."""
import sys
from pathlib import Path

tree, name = Path(sys.argv[1]), sys.argv[2]
path = tree / "crates/native/src/worker_timers.rs"
text = path.read_text()

def sub(old, new):
    global text
    assert text.count(old) == 1, old
    text = text.replace(old, new)

if name == "seq":
    # Deadline order under one lock hold.
    sub("""            queue.pop_expired_batch(now, expired);""", """            while expired.len() < expired.capacity() {
                let Some(waker) = queue.pop_expired(now) else { break };
                expired.push(waker);
            }""")
elif name == "inter":
    # The old pass: pop one timer, unlock, wake it, repeat, with one clock reading per pass.
    sub("""            queue.pop_expired_batch(now, expired);""", """            if let Some(waker) = queue.pop_expired(now) {
                expired.push(waker);
                let (_, wake_monitor) = refresh(shared, worker, &mut queue);
                drop(queue);
                if wake_monitor {
                    shared.monitor.unpark();
                }
                fired += 1;
                for waker in expired.drain(..) {
                    crate::panic::contain(|| waker.wake());
                }
                loop {
                    let mut queue = lock(worker);
                    let next = queue.pop_expired(now);
                    let (_, wake_monitor) = refresh(shared, worker, &mut queue);
                    drop(queue);
                    if wake_monitor {
                        shared.monitor.unpark();
                    }
                    let Some(waker) = next else { return fired };
                    fired += 1;
                    crate::panic::contain(|| waker.wake());
                }
            }""")
elif name.startswith("batch"):
    size = int(name[5:])
    sub("const BATCH: usize = 256;", f"const BATCH: usize = {size};")
else:
    sys.exit("unknown variant")
path.write_text(text)
