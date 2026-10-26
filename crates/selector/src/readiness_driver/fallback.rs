// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Platforms without edge polling retain independent async-io progress.

use crate::{SelectorDriver, SelectorWorker};
use std::{io, marker::PhantomData, rc::Rc, task::Waker, time::Duration};

/// A retired readiness driver on platforms without worker-owned edge polling.
pub struct ReadinessDriver {
    _private: (),
}

pub(crate) fn create(workers: usize) -> Vec<ReadinessDriver> {
    (0..workers)
        .map(|_| ReadinessDriver { _private: () })
        .collect()
}

/// A thread-bound retired worker; socket progress remains independently driven.
pub struct ReadinessWorker {
    _thread: PhantomData<Rc<()>>,
}
impl SelectorDriver for ReadinessDriver {
    type Worker = ReadinessWorker;
    fn enter(self) -> Self::Worker {
        ReadinessWorker {
            _thread: PhantomData,
        }
    }
}
impl SelectorWorker for ReadinessWorker {
    fn waker(&self) -> Waker {
        Waker::noop().clone()
    }
    fn close(&mut self) {}
    fn turn(&mut self, _timeout: Option<Duration>) -> io::Result<bool> {
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ReadinessSelector, SelectorOptions};

    #[test]
    fn retired_workers_preserve_independent_progress_and_validate_admission() {
        assert!(ReadinessSelector::with_workers(SelectorOptions::default(), 0).is_err());
        assert!(
            ReadinessSelector::with_workers(
                SelectorOptions {
                    threads: 0,
                    ..SelectorOptions::default()
                },
                1
            )
            .is_err()
        );
        let (selector, drivers) =
            ReadinessSelector::with_workers(SelectorOptions::default(), 2).unwrap();
        assert_eq!(drivers.len(), 2);
        for driver in drivers {
            let mut worker = driver.enter();
            worker.waker().wake_by_ref();
            assert!(worker.turn(None).unwrap());
            worker.close();
            assert!(worker.turn(Some(Duration::ZERO)).unwrap());
        }
        let socket =
            crate::NativeSocket::new(std::net::UdpSocket::bind("127.0.0.1:0").unwrap().into())
                .unwrap();
        let cancellation = base_scheduler::Cancellation::never();
        let mut read = Box::pin(selector.socket_read(&socket, vec![0; 1], &cancellation));
        assert!(
            futures_lite::future::block_on(futures_lite::future::poll_once(&mut read)).is_none()
        );
        selector.close();
        let (result, buffer) = futures_lite::future::block_on(read);
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(buffer, [0]);
        futures_lite::future::block_on(selector.shutdown());
    }
}
