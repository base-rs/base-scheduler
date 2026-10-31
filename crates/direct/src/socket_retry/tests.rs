// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use event_listener::Listener;
use futures_lite::future::{block_on, poll_once};

#[test]
fn retries_wake_without_workers_and_cancellation_and_close_do_not_lose_wakeups() {
    let retry = SocketRetry::new();
    let idle = retry.shared.idle.listen();
    block_on(retry.wait(&Cancellation::never())).unwrap();
    assert!(
        idle.wait_timeout(Duration::from_secs(5)).is_some(),
        "retry worker did not sleep while idle"
    );
    let token = Cancellation::new();
    let mut waiting = Box::pin(retry.wait(&token));
    if let Some(result) = block_on(poll_once(&mut waiting)) {
        result.unwrap();
    } else {
        token.cancel();
        assert!(matches!(
            block_on(waiting),
            Err(base_scheduler::OperationError::Cancelled(_))
        ));
    }
    token.cancel();
    assert!(matches!(
        block_on(retry.wait(&token)),
        Err(base_scheduler::OperationError::Cancelled(_))
    ));
    let never = Cancellation::never();
    let mut waiting = Box::pin(retry.wait(&never));
    if let Some(result) = block_on(poll_once(&mut waiting)) {
        result.unwrap();
        retry.close();
    } else {
        retry.close();
        block_on(waiting).unwrap();
    }
    assert_eq!(
        block_on(retry.wait(&never)).unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
}

#[test]
fn waiters_registered_while_the_worker_sleeps_are_woken_promptly() {
    let retry = Arc::new(SocketRetry::new());
    for _ in 0..100 {
        let idle = retry.shared.idle.listen();
        let (done, completion) = std::sync::mpsc::channel();
        let waiting = Arc::clone(&retry);
        thread::spawn(move || {
            let _ = done.send(block_on(waiting.wait(&Cancellation::never())));
        });
        completion
            .recv_timeout(Duration::from_secs(5))
            .expect("a waiter registered while the retry worker was idle was not woken")
            .unwrap();
        // The next waiter registers only after the worker has entered its indefinite wait.
        assert!(
            idle.wait_timeout(Duration::from_secs(5)).is_some(),
            "retry worker did not sleep while idle"
        );
    }
}

#[test]
fn retry_startup_failure_is_reported_consistently() {
    let mut retry = SocketRetry::new();
    retry.fail_start = true;
    for _ in 0..2 {
        assert_eq!(
            block_on(retry.wait(&Cancellation::never()))
                .unwrap_err()
                .to_string(),
            "injected socket retry startup failure"
        );
    }
}
