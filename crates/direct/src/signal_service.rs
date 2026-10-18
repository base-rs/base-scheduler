// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{DirectScheduler, DirectSubscription, thread};
use event_listener::Event;
use protocol_scheduler::{Notify, Signals};
use signal_hook::iterator::{Handle, Signals as NativeSignals};
use std::{
    collections::BTreeMap,
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex},
};

pub(crate) type Entry = (Vec<i32>, Arc<dyn Notify<i32>>);
pub(crate) struct State {
    closed: bool,
    next: usize,
    pub(crate) targets: BTreeMap<usize, Entry>,
    exited: bool,
}
pub(crate) struct Shared {
    pub(crate) state: Mutex<State>,
    changed: Event,
}
pub(crate) struct SignalService {
    pub(crate) shared: Arc<Shared>,
    handle: Mutex<Option<Handle>>,
}
impl Shared {
    pub(crate) fn dispatch(&self, signal: i32) {
        let targets: Vec<_> = self
            .state
            .lock()
            .unwrap()
            .targets
            .values()
            .filter(|(signals, _)| signals.contains(&signal))
            .map(|(_, target)| Arc::clone(target))
            .collect();
        for target in targets {
            // Each snapshot admits its callbacks under the registry lock, then executes user code without that lock or a raw OS handler.
            let _ = catch_unwind(AssertUnwindSafe(|| target.notify(signal)));
        }
    }
}
impl SignalService {
    pub(crate) fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    closed: false,
                    next: 0,
                    targets: BTreeMap::new(),
                    exited: true,
                }),
                changed: Event::new(),
            }),
            handle: Mutex::new(None),
        }
    }
    pub(crate) fn subscribe<T: Notify<i32> + 'static>(
        &self,
        signals: &[i32],
        target: T,
    ) -> io::Result<DirectSubscription> {
        if signals.is_empty()
            || signals.iter().any(|signal| {
                !(1..128).contains(signal) || signal_hook::consts::FORBIDDEN.contains(signal)
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "empty, invalid, or forbidden signal set",
            ));
        }
        let mut handle = self.handle.lock().unwrap();
        if self.shared.state.lock().unwrap().closed {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "signal service is closed",
            ));
        }
        if let Some(handle) = &*handle {
            for &signal in signals {
                handle.add_signal(signal)?;
            }
        } else {
            let mut native = NativeSignals::new(signals)?;
            let native_handle = native.handle();
            let shared = Arc::clone(&self.shared);
            shared.state.lock().unwrap().exited = false;
            if let Err(error) = thread::start("protocol-direct-signal", move || {
                for signal in native.forever() {
                    shared.dispatch(signal);
                }
                shared.state.lock().unwrap().exited = true;
                shared.changed.notify(usize::MAX);
            }) {
                self.shared.state.lock().unwrap().exited = true;
                return Err(error);
            }
            *handle = Some(native_handle);
        }
        let mut state = self.shared.state.lock().unwrap();
        let id = state.next;
        state.next = id
            .checked_add(1)
            .ok_or_else(|| io::Error::other("signal subscription identifiers exhausted"))?;
        state
            .targets
            .insert(id, (signals.to_vec(), Arc::new(target)));
        Ok(DirectSubscription {
            shared: Arc::downgrade(&self.shared),
            id,
        })
    }
    pub(crate) fn close(&self) {
        let handle = self.handle.lock().unwrap();
        let targets = {
            let mut state = self.shared.state.lock().unwrap();
            state.closed = true;
            std::mem::take(&mut state.targets)
        };
        if let Some(handle) = &*handle {
            handle.close();
        }
        drop(handle);
        drop(targets);
    }
    pub(crate) async fn drain(&self) {
        loop {
            event_listener::listener!(self.shared.changed => listener);
            if self.shared.state.lock().unwrap().exited {
                return;
            }
            listener.await;
        }
    }
}
impl Drop for SignalService {
    fn drop(&mut self) {
        self.close();
    }
}
impl Signals for DirectScheduler {
    type Signal = i32;
    type Subscription = DirectSubscription;
    fn signal_subscribe<T: Notify<i32> + 'static>(
        &self,
        signals: &[i32],
        target: T,
    ) -> io::Result<Self::Subscription> {
        self.signals.subscribe(signals, target)
    }
}

#[cfg(test)]
mod tests;
