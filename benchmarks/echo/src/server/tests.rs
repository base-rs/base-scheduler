// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use base_scheduler::*;
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
impl PartialSocket {
    fn new(input: impl IntoIterator<Item = Vec<u8>>, max_write: usize) -> Self {
        Self {
            input: Mutex::new(input.into_iter().collect()),
            output: Mutex::new(Vec::new()),
            reads: Mutex::new(Vec::new()),
            max_write,
        }
    }
    fn next_input(&self) -> Vec<u8> {
        self.input.lock().unwrap().pop_front().unwrap_or_default()
    }
    // Deliver the next queued input across the ranges in order.
    fn scatter<'b>(&self, ranges: impl IntoIterator<Item = &'b mut [u8]>) -> usize {
        let input = self.next_input();
        let mut rest = &input[..];
        for range in ranges {
            fill(&mut rest, range);
        }
        assert!(rest.is_empty(), "test input exceeds the read storage");
        input.len()
    }
    // Accept at most `max_write` bytes from the ranges in order.
    fn gather<'b>(&self, ranges: impl IntoIterator<Item = &'b [u8]>) -> usize {
        let mut output = self.output.lock().unwrap();
        let mut written = 0;
        for range in ranges {
            let count = range.len().min(self.max_write - written);
            output.extend_from_slice(&range[..count]);
            written += count;
        }
        written
    }
}
// Copy the front of `rest` into `range`, consuming what was copied.
fn fill(rest: &mut &[u8], range: &mut [u8]) {
    let count = range.len().min(rest.len());
    range[..count].copy_from_slice(&rest[..count]);
    *rest = &rest[count..];
}
struct Reader<'a>(&'a PartialSocket);
impl SocketStreamReader for Reader<'_> {
    type Buffer = BufferLease;
    async fn read(&mut self, cancellation: &Cancellation) -> OperationResult<Option<BufferLease>> {
        let input = self.0.next_input();
        if input.is_empty() {
            return Ok(None);
        }
        let pool = self.0.buffer_pool(BufferPoolOptions::new(1, input.len()))?;
        let mut chunk = pool.acquire(cancellation).await?;
        chunk.as_mut().copy_from_slice(&input);
        Ok(Some(chunk))
    }
    async fn read_into(&mut self, buffer: &mut [u8], _: &Cancellation) -> OperationResult<usize> {
        self.0.reads.lock().unwrap().push(buffer.as_ptr() as usize);
        let bytes = self.0.input.lock().unwrap().pop_front().unwrap_or_default();
        buffer[..bytes.len()].copy_from_slice(&bytes);
        Ok(bytes.len())
    }
    fn try_read_into(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::ErrorKind::Unsupported.into())
    }
    async fn read_vectored(
        &mut self,
        buffers: &mut [io::IoSliceMut<'_>],
        _: &Cancellation,
    ) -> OperationResult<usize> {
        self.0
            .reads
            .lock()
            .unwrap()
            .push(buffers[0].as_ptr() as usize);
        Ok(self
            .0
            .scatter(buffers.iter_mut().map(|buffer| &mut **buffer)))
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
        mut buffers: B,
        _: SocketRecvFlags,
        _: &Cancellation,
    ) -> (OperationResult<SocketRecvResult>, B) {
        self.0
            .reads
            .lock()
            .unwrap()
            .push(buffers.buffer(0).as_ptr() as usize);
        let input = self.0.next_input();
        let mut rest = &input[..];
        for index in 0..buffers.buffer_count() {
            fill(&mut rest, buffers.buffer_mut(index));
        }
        assert!(rest.is_empty(), "test input exceeds the read storage");
        let bytes = input.len();
        (
            Ok(SocketRecvResult {
                bytes,
                address: None,
                truncated: false,
                control_truncated: false,
            }),
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
        buffers: &[io::IoSlice<'_>],
        _: &Cancellation,
    ) -> OperationResult<usize> {
        Ok(self.0.gather(buffers.iter().map(|buffer| &**buffer)))
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
        let written = self
            .0
            .gather((0..buffers.buffer_count()).map(|index| buffers.buffer(index)));
        (Ok(written), buffers)
    }
    async fn close(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl SocketStreamReading for PartialSocket {
    fn socket_stream_reader<'a>(
        &'a self,
        _: &(),
    ) -> io::Result<impl SocketStreamReader<Buffer = BufferLease> + 'a + use<'a>> {
        Ok(Reader(self))
    }
    fn socket_stream_reader_with_pool<'a>(
        &'a self,
        _: &(),
        _: Self::Pool,
    ) -> io::Result<impl SocketStreamReader<Buffer = BufferLease> + 'a + use<'a>> {
        Ok(Reader(self))
    }
}
impl SocketStreamWriting for PartialSocket {
    fn socket_stream_writer<'a>(
        &'a self,
        _: &(),
    ) -> io::Result<impl SocketStreamWriter<Buffer = BufferLease> + 'a + use<'a>> {
        Ok(Writer(self, PhantomData))
    }
    fn socket_stream_writer_with_buffer<'a, B: IoBuf>(
        &'a self,
        _: &(),
    ) -> io::Result<impl SocketStreamWriter<Buffer = B> + 'a + use<'a, B>> {
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

// Spans both halves of the vectored modes' storage.
fn spanning_input() -> Vec<u8> {
    (0..HALF + 3).map(|index| index as u8).collect()
}

#[test]
fn vectored_echo_spans_both_halves_and_completes_partial_writes() {
    let scheduler = PartialSocket::new([b"abcdef".to_vec(), spanning_input()], 4099);
    crate::runtime(1).block_on(echo_vectored(&scheduler, ()));
    assert_eq!(
        *scheduler.output.lock().unwrap(),
        [b"abcdef".to_vec(), spanning_input()].concat()
    );
    let reads = scheduler.reads.lock().unwrap();
    assert_eq!(reads.len(), 3);
    assert!(reads.iter().all(|pointer| *pointer == reads[0]));
}

#[test]
fn vectored_echo_rejects_zero_progress() {
    let scheduler = PartialSocket::new([b"x".to_vec()], 0);
    assert!(
        std::panic::catch_unwind(|| crate::runtime(1).block_on(echo_vectored(&scheduler, ())))
            .is_err()
    );
}

#[test]
fn owned_vectored_echo_spans_both_halves_and_reuses_its_buffers() {
    let scheduler = PartialSocket::new([b"abcdef".to_vec(), spanning_input()], 4099);
    crate::runtime(1).block_on(echo_owned_vectored(&scheduler, ()));
    assert_eq!(
        *scheduler.output.lock().unwrap(),
        [b"abcdef".to_vec(), spanning_input()].concat()
    );
    let reads = scheduler.reads.lock().unwrap();
    assert_eq!(reads.len(), 3);
    assert!(reads.iter().all(|pointer| *pointer == reads[0]));
}

#[test]
fn owned_vectored_echo_rejects_zero_progress() {
    let scheduler = PartialSocket::new([b"x".to_vec()], 0);
    assert!(
        std::panic::catch_unwind(|| {
            crate::runtime(1).block_on(echo_owned_vectored(&scheduler, ()))
        })
        .is_err()
    );
}

#[test]
fn pooled_echo_completes_partial_writes_of_each_chunk() {
    let scheduler = PartialSocket::new([b"abcdef".to_vec(), b"xyz".to_vec()], 2);
    crate::runtime(1).block_on(echo_pooled(&scheduler, ()));
    assert_eq!(&*scheduler.output.lock().unwrap(), b"abcdefxyz");
}

#[test]
fn pooled_echo_rejects_zero_progress() {
    let scheduler = PartialSocket::new([b"x".to_vec()], 0);
    assert!(
        std::panic::catch_unwind(|| crate::runtime(1).block_on(echo_pooled(&scheduler, ())))
            .is_err()
    );
}
