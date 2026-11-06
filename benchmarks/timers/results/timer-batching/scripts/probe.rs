// Experiment: per-thread record of the latest expiry pass, read by the diagnostic.
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

thread_local! {
    static PASS_AT: Cell<Option<Instant>> = const { Cell::new(None) };
    static PASS_IDLE: Cell<Option<Instant>> = const { Cell::new(None) };
    static IDLE_WAKE: Cell<Option<Instant>> = const { Cell::new(None) };
}

static PASSES: AtomicU64 = AtomicU64::new(0);
static IDLE_PASSES: AtomicU64 = AtomicU64::new(0);
static FIRED: AtomicU64 = AtomicU64::new(0);
static SLEEPS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn start_pass() {
    PASS_IDLE.with(|p| p.set(IDLE_WAKE.with(|w| w.take())));
}
pub(crate) fn collect(now: Instant) {
    PASS_AT.with(|p| p.set(Some(now)));
}
#[repr(align(128))]
struct Padded(AtomicU64);
static BY_WORKER: [Padded; 16] = [const { Padded(AtomicU64::new(0)) }; 16];
pub(crate) fn fired_by(owner: usize, count: usize) {
    BY_WORKER[owner.min(15)].0.fetch_add(count as u64, Relaxed);
}
/// Timers fired by each worker's own passes.
pub fn by_worker() -> Vec<u64> {
    BY_WORKER.iter().map(|c| c.0.load(Relaxed)).collect()
}
pub(crate) fn fired(count: usize) {
    if count > 0 {
        PASSES.fetch_add(1, Relaxed);
        FIRED.fetch_add(count as u64, Relaxed);
        if PASS_IDLE.with(|p| p.get()).is_some() {
            IDLE_PASSES.fetch_add(1, Relaxed);
        }
    }
}
pub(crate) fn woke() {
    SLEEPS.fetch_add(1, Relaxed);
    IDLE_WAKE.with(|w| w.set(Some(Instant::now())));
}
/// The current thread's latest collecting pass and the idle wake that preceded it, if any.
pub fn get() -> (Option<Instant>, Option<Instant>) {
    (PASS_AT.with(|p| p.get()), PASS_IDLE.with(|p| p.get()))
}
/// Passes that fired, of which followed an idle wake, timers fired, and idle sleeps.
pub fn counts() -> [u64; 4] {
    [PASSES.load(Relaxed), IDLE_PASSES.load(Relaxed), FIRED.load(Relaxed), SLEEPS.load(Relaxed)]
}
