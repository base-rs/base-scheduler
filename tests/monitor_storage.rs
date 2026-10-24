// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::{backend::MonitorState, backend::MonitorStorage};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct Connection {
    state: AtomicUsize,
    drops: Arc<AtomicUsize>,
}
impl MonitorState for Connection {}

impl Drop for Connection {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn embedded_state_retains_its_containing_allocation_across_threads() {
    let drops = Arc::new(AtomicUsize::new(0));
    let connection = Arc::new(Connection {
        state: AtomicUsize::new(1),
        drops: Arc::clone(&drops),
    });
    let state = MonitorStorage::from_arc(Arc::clone(&connection), |connection| connection);
    assert_eq!(state.as_ptr(), &*connection as *const _);
    drop(connection);
    let copy = state.clone();
    std::thread::spawn(move || copy.state.fetch_add(1, Ordering::Relaxed))
        .join()
        .unwrap();
    assert_eq!(state.state.load(Ordering::Relaxed), 2);
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    drop(state);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    let state = MonitorStorage::new(Connection {
        state: AtomicUsize::new(3),
        drops,
    });
    assert_eq!(state.state.load(Ordering::Relaxed), 3);
}

#[test]
fn socket_monitor_state_preserves_its_resource_and_direction() {
    use core_scheduler::{Interest, backend::SocketMonitor};
    let monitor = SocketMonitor::new(42, Interest::Readable, "registration");
    assert_eq!(*monitor.socket(), 42);
    assert_eq!(*monitor.registration(), "registration");
    assert_eq!(monitor.interest(), Interest::Readable);
    monitor.check_interest(Interest::Readable).unwrap();
    assert_eq!(
        monitor
            .check_interest(Interest::Writable)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert_eq!(monitor.into_parts(), (42, "registration"));
}

#[test]
fn backend_retention_does_not_extend_application_handle_lifetime() {
    struct State {
        users: AtomicUsize,
    }
    impl MonitorState for State {
        fn acquire(&self) {
            self.users.fetch_add(1, Ordering::Relaxed);
        }
        fn release(&self) {
            self.users.fetch_sub(1, Ordering::Relaxed);
        }
    }
    let application = MonitorStorage::new(State {
        users: AtomicUsize::new(0),
    });
    let alias = application.clone();
    let backend = application.retained();
    let copy = backend.clone();
    assert_eq!(backend.users.load(Ordering::Relaxed), 2);
    drop(application);
    drop(alias);
    assert_eq!(backend.users.load(Ordering::Relaxed), 0);
    drop(copy);
    assert_eq!(backend.users.load(Ordering::Relaxed), 0);
}

#[test]
fn stream_state_projects_the_same_registration_without_allocating() {
    use core_scheduler::{
        Interest, backend::SocketMonitor, backend::SocketMonitorState, backend::SocketStreamMonitor,
    };
    struct Registration(Arc<AtomicUsize>);
    impl MonitorState for Registration {
        fn acquire(&self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        fn release(&self) {
            self.0.fetch_sub(1, Ordering::Relaxed);
        }
    }
    struct StreamConnection {
        read: SocketStreamMonitor<usize, Registration>,
        // Ensure the projected monitor is actually a subobject, not necessarily the allocation address.
        _other: [usize; 4],
    }
    let users = Arc::new(AtomicUsize::new(0));
    let connection = Arc::new(StreamConnection {
        read: SocketStreamMonitor::new(SocketMonitor::new(
            42,
            Interest::Readable,
            Registration(Arc::clone(&users)),
        )),
        _other: [0; 4],
    });
    let weak = Arc::downgrade(&connection);
    let application = MonitorStorage::from_arc(connection, |connection| &connection.read);
    assert_eq!(*application.socket(), 42);
    assert_eq!(application.interest(), Interest::Readable);
    assert_eq!(application.registration().0.load(Ordering::Relaxed), 1);
    application.check_interest(Interest::Readable).unwrap();
    let alias = application.clone();
    assert_eq!(users.load(Ordering::Relaxed), 2);
    let counts = allocation_counter::measure(|| {
        let backend = application.retained_project(AsRef::as_ref);
        assert_eq!(backend.as_ptr(), application.as_ref() as *const _);
        assert_eq!(*backend.socket(), 42);
        let copy = backend.clone();
        drop(copy);
        drop(backend);
    });
    assert_eq!(counts.count_total, 0);
    let backend = application.retained_project(AsRef::as_ref);
    drop(application);
    drop(alias);
    assert_eq!(users.load(Ordering::Relaxed), 0);
    assert!(weak.upgrade().is_some());
    drop(backend);
    assert!(weak.upgrade().is_none());
    assert_eq!(
        std::mem::size_of::<SocketStreamMonitor<usize, Registration>>(),
        std::mem::size_of::<SocketMonitor<usize, Registration>>()
    );
    fn stream_policy<M: SocketMonitorState<usize, Registration>>() -> bool {
        M::STREAM
    }
    assert!(!stream_policy::<SocketMonitor<usize, Registration>>());
    assert!(stream_policy::<SocketStreamMonitor<usize, Registration>>());
}
