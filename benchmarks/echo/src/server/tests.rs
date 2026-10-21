// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use core_scheduler::{BufferResult, IoBufMut, OperationResult, SocketOperation};
use std::{collections::VecDeque, sync::Mutex};

struct PartialSocket {
    input: Mutex<VecDeque<Vec<u8>>>,
    output: Mutex<Vec<u8>>,
    reads: Mutex<Vec<usize>>,
    max_write: usize,
}
impl SocketOperation for PartialSocket {
    type Socket = ();
}
impl SocketRead for PartialSocket {
    async fn socket_read<B: IoBufMut>(
        &self,
        _: &(),
        mut buffer: B,
        _: &Cancellation,
    ) -> BufferResult<B> {
        self.reads
            .lock()
            .unwrap()
            .push(buffer.as_ref().as_ptr() as usize);
        let bytes = self.input.lock().unwrap().pop_front().unwrap_or_default();
        buffer.as_mut()[..bytes.len()].copy_from_slice(&bytes);
        (Ok(bytes.len()), buffer)
    }
}
impl SocketWrite for PartialSocket {
    async fn socket_write<B: IoBuf>(&self, _: &(), buffer: B, _: &Cancellation) -> BufferResult<B> {
        let bytes = &buffer.as_ref()[..buffer.as_ref().len().min(self.max_write)];
        self.output.lock().unwrap().extend_from_slice(bytes);
        (Ok(bytes.len()), buffer)
    }
}

impl SocketRecvPoll for PartialSocket {
    async fn socket_recv_poll(
        &self,
        _: &(),
        buffer: &mut [u8],
        _: SocketRecvFlags,
        _: &Cancellation,
    ) -> OperationResult<usize> {
        self.reads.lock().unwrap().push(buffer.as_ptr() as usize);
        let bytes = self.input.lock().unwrap().pop_front().unwrap_or_default();
        buffer[..bytes.len()].copy_from_slice(&bytes);
        Ok(bytes.len())
    }
}

impl SocketSendPoll for PartialSocket {
    async fn socket_send_poll(
        &self,
        _: &(),
        buffer: &[u8],
        _: SocketSendFlags,
        _: &Cancellation,
    ) -> OperationResult<usize> {
        let bytes = &buffer[..buffer.len().min(self.max_write)];
        self.output.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
}

#[test]
fn echo_reuses_the_allocation_and_completes_partial_writes() {
    let scheduler = PartialSocket {
        input: Mutex::new([b"abcdef".to_vec(), b"xyz".to_vec()].into()),
        output: Mutex::new(Vec::new()),
        reads: Mutex::new(Vec::new()),
        max_write: 2,
    };
    crate::runtime(1).block_on(echo(&scheduler, ()));
    assert_eq!(&*scheduler.output.lock().unwrap(), b"abcdefxyz");
    let reads = scheduler.reads.lock().unwrap();
    assert_eq!(reads.len(), 3);
    assert!(reads.iter().all(|pointer| *pointer == reads[0]));
}

#[test]
fn zero_progress_is_an_error_instead_of_an_infinite_loop() {
    let scheduler = PartialSocket {
        input: Mutex::new([b"x".to_vec()].into()),
        output: Mutex::new(Vec::new()),
        reads: Mutex::new(Vec::new()),
        max_write: 0,
    };
    assert!(std::panic::catch_unwind(|| crate::runtime(1).block_on(echo(&scheduler, ()))).is_err());
}

#[test]
fn poll_echo_reuses_the_allocation_and_completes_partial_writes() {
    let scheduler = PartialSocket {
        input: Mutex::new([b"abcdef".to_vec(), b"xyz".to_vec()].into()),
        output: Mutex::new(Vec::new()),
        reads: Mutex::new(Vec::new()),
        max_write: 2,
    };
    crate::runtime(1).block_on(echo_poll(&scheduler, ()));
    assert_eq!(&*scheduler.output.lock().unwrap(), b"abcdefxyz");
    let reads = scheduler.reads.lock().unwrap();
    assert_eq!(reads.len(), 3);
    assert!(reads.iter().all(|pointer| *pointer == reads[0]));
}

#[test]
fn poll_zero_progress_is_an_error_instead_of_an_infinite_loop() {
    let scheduler = PartialSocket {
        input: Mutex::new([b"x".to_vec()].into()),
        output: Mutex::new(Vec::new()),
        reads: Mutex::new(Vec::new()),
        max_write: 0,
    };
    assert!(
        std::panic::catch_unwind(|| crate::runtime(1).block_on(echo_poll(&scheduler, ()))).is_err()
    );
}

impl core_scheduler::SocketWait for PartialSocket {
    async fn socket_wait(
        &self,
        _: &(),
        _: core_scheduler::Interest,
        _: &Cancellation,
    ) -> OperationResult<()> {
        Ok(())
    }
}
impl core_scheduler::SocketRecvMsgPoll for PartialSocket {
    async fn socket_recvmsg_poll(
        &self,
        _: &(),
        _: &mut [std::io::IoSliceMut<'_>],
        _: SocketRecvFlags,
        _: &Cancellation,
    ) -> OperationResult<core_scheduler::SocketRecvResult> {
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported).into())
    }
}
impl core_scheduler::SocketSendMsgPoll for PartialSocket {
    async fn socket_sendmsg_poll(
        &self,
        _: &(),
        _: &[std::io::IoSlice<'_>],
        _: Option<core_scheduler::SocketAddress>,
        _: SocketSendFlags,
        _: &Cancellation,
    ) -> OperationResult<usize> {
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported).into())
    }
}
impl SocketMonitoring for PartialSocket {
    type MonitorRegistration = ();
    fn socket_monitor_state(
        &self,
        _: &(),
        interest: core_scheduler::Interest,
    ) -> std::io::Result<core_scheduler::SocketMonitor<(), ()>> {
        Ok(core_scheduler::SocketMonitor::new((), interest, ()))
    }
}

#[test]
fn monitor_echo_reuses_payload_storage_and_completes_partial_writes() {
    let scheduler = PartialSocket {
        input: Mutex::new([b"abcdef".to_vec(), b"xyz".to_vec()].into()),
        output: Mutex::new(Vec::new()),
        reads: Mutex::new(Vec::new()),
        max_write: 2,
    };
    crate::runtime(1).block_on(echo_monitor(&scheduler, ()));
    assert_eq!(&*scheduler.output.lock().unwrap(), b"abcdefxyz");
    let reads = scheduler.reads.lock().unwrap();
    assert_eq!(reads.len(), 3);
    assert!(reads.iter().all(|pointer| *pointer == reads[0]));
}

#[test]
fn monitor_echo_rejects_zero_progress() {
    let scheduler = PartialSocket {
        input: Mutex::new([b"x".to_vec()].into()),
        output: Mutex::new(Vec::new()),
        reads: Mutex::new(Vec::new()),
        max_write: 0,
    };
    assert!(
        std::panic::catch_unwind(|| crate::runtime(1).block_on(echo_monitor(&scheduler, ())))
            .is_err()
    );
}
