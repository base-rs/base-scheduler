// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Production calls libc directly. Tests inject only selected native operations on their own thread.

#[cfg(not(test))]
pub(super) use libc::{
    epoll_create1, epoll_ctl, epoll_pwait, eventfd, read, timerfd_create, timerfd_settime, write,
};

#[cfg(test)]
pub(super) use fault::{
    epoll_create1, epoll_ctl, epoll_pwait, eventfd, read, timerfd_create, timerfd_settime, write,
};

// Keep compatibility resource/wait failures deterministic without affecting native production calls.
pub(super) fn compatible_new() -> std::io::Result<polling::Poller> {
    #[cfg(test)]
    fault::check(fault::Call::CompatibleCreate)?;
    polling::Poller::new()
}
pub(super) fn compatible_wait(
    poller: &polling::Poller,
    events: &mut polling::Events,
    timeout: Option<std::time::Duration>,
) -> std::io::Result<usize> {
    #[cfg(test)]
    fault::check(fault::Call::CompatibleWait)?;
    poller.wait(events, timeout)
}

#[cfg(test)]
pub(super) mod fault {
    use std::{cell::RefCell, collections::VecDeque};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(in super::super) enum Call {
        Queue,
        Interrupt,
        Timer,
        Control,
        Wait,
        Read,
        Write,
        SetTime,
        CompatibleCreate,
        CompatibleWait,
    }
    thread_local! {
        static PLAN: RefCell<VecDeque<(Call, i32)>> = const { RefCell::new(VecDeque::new()) };
    }
    pub(in super::super) fn plan(entries: &[(Call, i32)]) {
        PLAN.with(|plan| {
            let mut plan = plan.borrow_mut();
            assert!(plan.is_empty(), "previous native failure was not consumed");
            plan.extend(entries.iter().copied());
        });
    }
    pub(in super::super) fn drained() {
        PLAN.with(|plan| assert!(plan.borrow().is_empty()));
    }
    fn take_error(call: Call) -> Option<i32> {
        PLAN.with(|plan| {
            let mut plan = plan.borrow_mut();
            if plan.front().is_some_and(|(next, _)| *next == call) {
                plan.pop_front().map(|(_, error)| error)
            } else {
                None
            }
        })
    }
    pub(in super::super) fn check(call: Call) -> std::io::Result<()> {
        match take_error(call) {
            Some(error) if error != 0 => Err(std::io::Error::from_raw_os_error(error)),
            _ => Ok(()),
        }
    }
    fn invoke(call: Call, operation: impl FnOnce() -> libc::c_long) -> libc::c_long {
        match take_error(call) {
            // A zero entry delegates one occurrence, allowing tests to fail a later call of the same operation.
            None | Some(0) => operation(),
            Some(error) => {
                // SAFETY: errno is thread-local on Linux.
                unsafe {
                    *libc::__errno_location() = error;
                }
                -1
            }
        }
    }
    pub(in super::super) unsafe fn epoll_create1(flags: i32) -> i32 {
        invoke(Call::Queue, || unsafe {
            libc::epoll_create1(flags) as libc::c_long
        }) as i32
    }
    pub(in super::super) unsafe fn eventfd(value: u32, flags: i32) -> i32 {
        invoke(Call::Interrupt, || unsafe {
            libc::eventfd(value, flags) as libc::c_long
        }) as i32
    }
    pub(in super::super) unsafe fn timerfd_create(clock: libc::clockid_t, flags: i32) -> i32 {
        invoke(Call::Timer, || unsafe {
            libc::timerfd_create(clock, flags) as libc::c_long
        }) as i32
    }
    pub(in super::super) unsafe fn epoll_ctl(
        queue: i32,
        operation: i32,
        descriptor: i32,
        event: *mut libc::epoll_event,
    ) -> i32 {
        invoke(Call::Control, || unsafe {
            libc::epoll_ctl(queue, operation, descriptor, event) as libc::c_long
        }) as i32
    }
    pub(in super::super) unsafe fn epoll_pwait(
        queue: i32,
        events: *mut libc::epoll_event,
        capacity: i32,
        timeout: i32,
        mask: *const libc::sigset_t,
    ) -> i32 {
        invoke(Call::Wait, || unsafe {
            libc::epoll_pwait(queue, events, capacity, timeout, mask) as libc::c_long
        }) as i32
    }
    pub(in super::super) unsafe fn read(
        descriptor: i32,
        output: *mut libc::c_void,
        count: usize,
    ) -> libc::ssize_t {
        invoke(Call::Read, || unsafe {
            libc::read(descriptor, output, count) as libc::c_long
        }) as libc::ssize_t
    }
    pub(in super::super) unsafe fn write(
        descriptor: i32,
        input: *const libc::c_void,
        count: usize,
    ) -> libc::ssize_t {
        invoke(Call::Write, || unsafe {
            libc::write(descriptor, input, count) as libc::c_long
        }) as libc::ssize_t
    }
    pub(in super::super) unsafe fn timerfd_settime(
        descriptor: i32,
        flags: i32,
        setting: *const libc::itimerspec,
        old: *mut libc::itimerspec,
    ) -> i32 {
        invoke(Call::SetTime, || unsafe {
            libc::timerfd_settime(descriptor, flags, setting, old) as libc::c_long
        }) as i32
    }
}
