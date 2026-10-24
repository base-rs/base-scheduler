// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::{collections::VecDeque, sync::Arc};

pub(crate) trait SubmissionNotification: Send + Sync {
    fn submitted(&self);
}

#[derive(Default)]
pub(super) struct SubmissionNotifications {
    consumed: usize,
    pending: VecDeque<(usize, Arc<dyn SubmissionNotification>)>,
}

impl SubmissionNotifications {
    pub fn pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn push(&mut self, queue_length: usize, notification: Arc<dyn SubmissionNotification>) {
        self.pending
            .push_back((self.consumed.wrapping_add(queue_length), notification));
    }

    pub fn consumed(&mut self, before: usize, after: usize) {
        self.consumed = self.consumed.wrapping_add(before - after);
        // Ring capacity is far below isize::MAX, so a signed wrapping distance distinguishes pending markers even after the cumulative counter wraps.
        while self
            .pending
            .front()
            .is_some_and(|(position, _)| position.wrapping_sub(self.consumed) as isize <= 0)
        {
            let (_, notification) = self.pending.pop_front().unwrap();
            notification.submitted();
        }
        if self.pending.is_empty() {
            self.consumed = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Notification(Arc<AtomicUsize>);
    impl SubmissionNotification for Notification {
        fn submitted(&self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn partial_submission_notifies_only_consumed_entries_even_with_later_refills() {
        let completed = Arc::new(AtomicUsize::new(0));
        let mut notices = SubmissionNotifications::default();
        notices.push(2, Arc::new(Notification(Arc::clone(&completed))));
        notices.push(4, Arc::new(Notification(Arc::clone(&completed))));
        notices.consumed(5, 4);
        assert_eq!(completed.load(Ordering::Relaxed), 0);
        notices.push(6, Arc::new(Notification(Arc::clone(&completed))));
        notices.consumed(6, 4);
        assert_eq!(completed.load(Ordering::Relaxed), 1);
        notices.consumed(4, 3);
        assert_eq!(completed.load(Ordering::Relaxed), 2);
        notices.consumed(3, 0);
        assert_eq!(completed.load(Ordering::Relaxed), 3);
        assert!(!notices.pending());
    }

    #[test]
    fn consumed_counter_wrap_preserves_order() {
        let completed = Arc::new(AtomicUsize::new(0));
        let mut notices = SubmissionNotifications {
            consumed: usize::MAX - 1,
            ..Default::default()
        };
        notices.push(3, Arc::new(Notification(Arc::clone(&completed))));
        notices.consumed(4, 2);
        assert_eq!(completed.load(Ordering::Relaxed), 0);
        notices.consumed(2, 1);
        assert_eq!(completed.load(Ordering::Relaxed), 1);
        assert!(!notices.pending());
    }

    #[test]
    fn issuance_is_acknowledged_after_kernel_entry_without_waiting_for_output_completion() {
        use super::super::{
            socket::retain_socket,
            tests::{Failures, drive, pair},
        };
        use core_scheduler::*;
        use futures_lite::future::{block_on, poll_once};
        use std::{io, time::Duration};

        let (selector, mut driver) = pair(16);
        let (sender, receiver) = block_on(core_scheduler_conformance::socket::stream_pair(
            &selector,
            SocketDomain::Ipv4,
        ));
        retain_socket(&sender)
            .get_ref()
            .set_send_buffer_size(4096)
            .unwrap();
        let mut writer = selector.socket_stream_writer::<Vec<u8>>(&sender).unwrap();
        let never = Cancellation::never();
        let mut submission = Box::pin(writer.submit(&selector, vec![1; 4 * 1024 * 1024], &never));
        assert!(block_on(poll_once(submission.as_mut())).is_none());
        {
            let _failure = Failures::new([("submit", io::ErrorKind::Interrupted.into())]);
            driver.turn(Some(Duration::ZERO)).unwrap();
        }
        assert!(
            block_on(poll_once(submission.as_mut())).is_none(),
            "staging without kernel entry must not acknowledge issuance"
        );
        // No peer is consuming bytes. A blocking completion wait here would deadlock issuance, so pending receipts force a nonblocking submission turn.
        driver.turn(None).unwrap();
        assert!(matches!(
            block_on(poll_once(submission.as_mut())),
            Some(Ok(()))
        ));
        drop(submission);
        let mut flush = Box::pin(writer.flush(&selector, &never));
        assert!(block_on(poll_once(flush.as_mut())).is_none());
        drop(flush);
        drive(&mut driver, writer.close(&selector)).unwrap();
        drop(writer);
        block_on(selector.socket_close(sender)).unwrap();
        block_on(selector.socket_close(receiver)).unwrap();
    }

    #[test]
    fn issuance_handles_full_submission_queues_and_abandoned_waiters() {
        use super::super::{
            socket::retain_socket,
            tests::drive,
        };
        use core_scheduler::*;
        use futures_lite::future::{block_on, poll_once};
        use std::time::Duration;

        let (selector, mut driver) = crate::IoUringSelector::new(crate::IoUringOptions {
            entries: 1,
            ..Default::default()
        })
        .unwrap();
        let (sender, receiver) = block_on(core_scheduler_conformance::socket::stream_pair(
            &selector,
            SocketDomain::Ipv4,
        ));
        let (other_sender, other_receiver) = block_on(
            core_scheduler_conformance::socket::stream_pair(&selector, SocketDomain::Ipv4),
        );
        retain_socket(&sender)
            .get_ref()
            .set_send_buffer_size(4096)
            .unwrap();
        retain_socket(&other_sender)
            .get_ref()
            .set_send_buffer_size(4096)
            .unwrap();
        let mut first = selector.socket_stream_writer::<Vec<u8>>(&sender).unwrap();
        let mut second = selector
            .socket_stream_writer::<Vec<u8>>(&other_sender)
            .unwrap();
        let never = Cancellation::never();
        let mut a = Box::pin(first.submit(&selector, vec![1; 4 * 1024 * 1024], &never));
        let mut b = Box::pin(second.submit(&selector, vec![2; 4 * 1024 * 1024], &never));
        assert!(block_on(poll_once(a.as_mut())).is_none());
        assert!(block_on(poll_once(b.as_mut())).is_none());
        driver.turn(Some(Duration::ZERO)).unwrap();
        assert!(matches!(block_on(poll_once(a.as_mut())), Some(Ok(()))));
        assert!(block_on(poll_once(b.as_mut())).is_none());
        drop(a);
        // Accepted storage remains owned by the second writer even when its receipt waiter is abandoned.
        drop(b);
        drive(&mut driver, first.close(&selector)).unwrap();
        drive(&mut driver, second.close(&selector)).unwrap();
        drop(first);
        drop(second);
        for socket in [sender, receiver, other_sender, other_receiver] {
            block_on(selector.socket_close(socket)).unwrap();
        }
    }

    #[test]
    fn terminal_failure_before_issuance_still_resolves_accepted_submission() {
        use super::super::tests::{drive, pair};
        use core_scheduler::*;
        use futures_lite::future::{block_on, poll_once};
        use std::time::Duration;

        let (selector, mut driver) = pair(16);
        let (sender, receiver) = block_on(core_scheduler_conformance::socket::stream_pair(
            &selector,
            SocketDomain::Ipv4,
        ));
        let mut writer = selector.socket_stream_writer::<Vec<u8>>(&sender).unwrap();
        let token = Cancellation::new();
        let mut submission = Box::pin(writer.submit(&selector, vec![1; 8 * 1024 * 1024], &token));
        assert!(block_on(poll_once(submission.as_mut())).is_none());
        token.cancel();
        driver.turn(Some(Duration::ZERO)).unwrap();
        assert!(matches!(
            block_on(poll_once(submission.as_mut())),
            Some(Ok(()))
        ));
        drop(submission);
        assert!(matches!(
            block_on(writer.flush(&selector, &Cancellation::never())),
            Err(OperationError::Cancelled(_))
        ));
        drive(&mut driver, writer.close(&selector)).unwrap();
        drop(writer);
        block_on(selector.socket_close(sender)).unwrap();
        block_on(selector.socket_close(receiver)).unwrap();
    }
}
