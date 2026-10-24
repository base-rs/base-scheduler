// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use core_scheduler::*;
use std::{collections::VecDeque, marker::PhantomData, sync::Mutex};

struct PartialSocket {
    input: Mutex<VecDeque<Vec<u8>>>,
    output: Mutex<Vec<u8>>,
    reads: Mutex<Vec<usize>>,
    max_write: usize,
}
impl SocketOperation for PartialSocket {
    type Socket = ();
}
impl BufferPooling for PartialSocket {
    type Pool = HeapBufferPool;
    fn buffer_pool(&self, options: BufferPoolOptions) -> io::Result<Self::Pool> {
        HeapBufferPool::with_options(options)
    }
}
struct Reader<'a>(&'a PartialSocket);
impl SocketStreamReader for Reader<'_> {
    type Buffer = BufferLease;
    async fn read(&mut self, _: &Cancellation) -> OperationResult<Option<BufferLease>> {
        Err(io::Error::from(io::ErrorKind::Unsupported).into())
    }
    async fn read_into(&mut self, buffer: &mut [u8], _: &Cancellation) -> OperationResult<usize> {
        self.0.reads.lock().unwrap().push(buffer.as_ptr() as usize);
        let bytes = self.0.input.lock().unwrap().pop_front().unwrap_or_default();
        buffer[..bytes.len()].copy_from_slice(&bytes);
        Ok(bytes.len())
    }
    async fn read_vectored(
        &mut self,
        _: &mut [io::IoSliceMut<'_>],
        _: &Cancellation,
    ) -> OperationResult<usize> {
        Err(io::Error::from(io::ErrorKind::Unsupported).into())
    }
    async fn read_vectored_with_flags(
        &mut self,
        _: &mut [io::IoSliceMut<'_>],
        _: SocketRecvFlags,
        _: &Cancellation,
    ) -> OperationResult<SocketRecvResult> {
        Err(io::Error::from(io::ErrorKind::Unsupported).into())
    }
    async fn read_owned_vectored<B: IoBufSequenceMut>(
        &mut self,
        buffers: B,
        _: SocketRecvFlags,
        _: &Cancellation,
    ) -> (OperationResult<SocketRecvResult>, B) {
        (
            Err(io::Error::from(io::ErrorKind::Unsupported).into()),
            buffers,
        )
    }
    async fn close(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct Writer<'a, B: IoBuf>(&'a PartialSocket, PhantomData<B>);
impl<B: IoBuf> SocketStreamWriter for Writer<'_, B> {
    type Buffer = B;
    async fn write(&mut self, buffer: &[u8], _: &Cancellation) -> OperationResult<usize> {
        let bytes = &buffer[..buffer.len().min(self.0.max_write)];
        self.0.output.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    async fn write_vectored(
        &mut self,
        _: &[io::IoSlice<'_>],
        _: &Cancellation,
    ) -> OperationResult<usize> {
        Err(io::Error::from(io::ErrorKind::Unsupported).into())
    }
    async fn submit(
        &mut self,
        _: B,
        _: &Cancellation,
    ) -> std::result::Result<(), (OperationError, B)> {
        unreachable!("The borrowed echo benchmark never submits owned output")
    }
    async fn flush(&mut self, _: &Cancellation) -> OperationResult<()> {
        Ok(())
    }
    async fn write_vectored_with_flags(
        &mut self,
        _: &[io::IoSlice<'_>],
        _: SocketSendFlags,
        _: &Cancellation,
    ) -> OperationResult<usize> {
        Err(io::Error::from(io::ErrorKind::Unsupported).into())
    }
    async fn write_owned_vectored<Buffers: IoBufSequence>(
        &mut self,
        buffers: Buffers,
        _: SocketSendFlags,
        _: &Cancellation,
    ) -> (OperationResult<usize>, Buffers) {
        (
            Err(io::Error::from(io::ErrorKind::Unsupported).into()),
            buffers,
        )
    }
    async fn close(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl SocketStreamReading for PartialSocket {
    fn socket_stream_reader<'a>(
        &'a self,
        _: &(),
    ) -> io::Result<impl SocketStreamReader<Buffer = BufferLease> + 'a> {
        Ok(Reader(self))
    }
    fn socket_stream_reader_with_pool<'a>(
        &'a self,
        _: &(),
        _: Self::Pool,
    ) -> io::Result<impl SocketStreamReader<Buffer = BufferLease> + 'a> {
        Ok(Reader(self))
    }
}
impl SocketStreamWriting for PartialSocket {
    fn socket_stream_writer<'a>(
        &'a self,
        _: &(),
    ) -> io::Result<impl SocketStreamWriter<Buffer = BufferLease> + 'a> {
        Ok(Writer(self, PhantomData))
    }
    fn socket_stream_writer_with_buffer<'a, B: IoBuf>(
        &'a self,
        _: &(),
    ) -> io::Result<impl SocketStreamWriter<Buffer = B> + 'a> {
        Ok(Writer(self, PhantomData))
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
