// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Adopted pipe assertions for [`PipeReading`] and [`PipeWriting`], using standard-library pipes and a `cat` child process.

use base_scheduler::{
    Cancellation, Cancelled, OperationError, PipeReader, PipeReading, PipeWriter, PipeWriting,
};
use futures_lite::future::{poll_once, zip};
use std::{
    io::{self, IoSlice, IoSliceMut, Read},
    os::fd::OwnedFd,
    pin::pin,
    process::{Command, Stdio},
};

// Spawning a child can briefly duplicate every descriptor in this process until the child closes its close-on-exec copies, which keeps a closed pipe end alive. End-of-file assertions tolerate the delay; broken-pipe assertions retry until any such copy is gone.
fn pipe() -> (OwnedFd, OwnedFd) {
    let (reader, writer) = io::pipe().unwrap();
    (reader.into(), writer.into())
}

// Write through a blocking standard-library alias until the read end is gone, returning the first error.
fn broken(mut probe: io::PipeWriter) -> io::Error {
    std::iter::repeat_with(|| {
        std::thread::yield_now();
        io::Write::write(&mut probe, b"x")
    })
    .find_map(Result::err)
    .unwrap()
}

fn assert_cancelled<T: std::fmt::Debug>(result: Result<T, OperationError>) {
    assert_eq!(
        std::mem::discriminant(&result.unwrap_err()),
        std::mem::discriminant(&OperationError::from(Cancelled))
    );
}

fn assert_broken_pipe<T: std::fmt::Debug>(result: Result<T, OperationError>) {
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
}

// Fill a nonblocking pipe until a write would wait, returning the bytes accepted.
async fn fill(writer: &mut impl PipeWriter) -> usize {
    let chunk = [0x5a; 16 * 1024];
    let cancellation = Cancellation::never();
    let mut total = 0;
    loop {
        match poll_once(pin!(writer.write(&chunk, &cancellation))).await {
            Some(result) => total += result.unwrap(),
            None => return total,
        }
    }
}

// Read until end of file, returning every byte observed.
async fn read_to_end(reader: &mut impl PipeReader) -> Vec<u8> {
    let cancellation = Cancellation::never();
    let mut output = Vec::new();
    let mut buffer = [0; 8 * 1024];
    loop {
        match reader.read_into(&mut buffer, &cancellation).await.unwrap() {
            0 => return output,
            count => output.extend_from_slice(&buffer[..count]),
        }
    }
}

/// Round-trip a payload larger than the pipe buffers through `cat`, adopting its `ChildStdin` and `ChildStdout`. Writing and reading proceed concurrently, so both directions must wait for readiness. Closing the writer delivers end of file to the child, whose exit closes its output.
pub async fn child_round_trip<S: PipeReading + PipeWriting>(scheduler: &S) {
    let mut child = Command::new("cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut writer = scheduler
        .pipe_writer(child.stdin.take().unwrap().into())
        .unwrap();
    let mut reader = scheduler
        .pipe_reader(child.stdout.take().unwrap().into())
        .unwrap();
    let payload: Vec<u8> = (0..1024 * 1024).map(|index| (index % 251) as u8).collect();
    let writing = async {
        let cancellation = Cancellation::never();
        let mut remaining = &payload[..];
        while !remaining.is_empty() {
            let count = writer.write(remaining, &cancellation).await.unwrap();
            remaining = &remaining[count..];
        }
        writer.close().await.unwrap();
    };
    let ((), output) = zip(writing, read_to_end(&mut reader)).await;
    assert!(output == payload, "cat must echo the complete payload");
    reader.close().await.unwrap();
    assert!(child.wait().unwrap().success());
}

/// Gather and scatter transfers, including waits for input and for output capacity.
pub async fn vectored<S: PipeReading + PipeWriting>(scheduler: &S) {
    let (input, output) = pipe();
    let mut reader = scheduler.pipe_reader(input).unwrap();
    let mut writer = scheduler.pipe_writer(output).unwrap();
    let cancellation = Cancellation::never();
    let mut first = [0; 4];
    let mut second = [0; 16];
    {
        let mut buffers = [IoSliceMut::new(&mut first), IoSliceMut::new(&mut second)];
        let mut reading = pin!(reader.read_vectored(&mut buffers, &cancellation));
        assert!(poll_once(reading.as_mut()).await.is_none());
        let written = writer
            .write_vectored(
                &[IoSlice::new(b"hello "), IoSlice::new(b"pipe")],
                &cancellation,
            )
            .await
            .unwrap();
        assert_eq!(written, 10);
        assert_eq!(reading.await.unwrap(), 10);
    }
    assert_eq!(&first, b"hell");
    assert_eq!(&second[..6], b"o pipe");

    let filled = fill(&mut writer).await;
    assert!(filled > 0);
    {
        let tail = [IoSlice::new(b"tail")];
        let mut writing = pin!(writer.write_vectored(&tail, &cancellation));
        assert!(poll_once(writing.as_mut()).await.is_none());
        // The pipe holds exactly the filled bytes. Draining them guarantees capacity for the pending write.
        let mut drained = 0;
        let mut buffer = vec![0; 64 * 1024];
        while drained < filled {
            drained += reader.read_into(&mut buffer, &cancellation).await.unwrap();
        }
        let written = writing.await.unwrap();
        assert_eq!(written, 4);
    }
    writer.close().await.unwrap();
    assert_eq!(read_to_end(&mut reader).await, b"tail");
    reader.close().await.unwrap();
}

/// A pending read observes end of file when the adopted writer is dropped, without an explicit close, and repeated reads continue to report it.
pub async fn end_of_file<S: PipeReading + PipeWriting>(scheduler: &S) {
    let (input, output) = pipe();
    let mut reader = scheduler.pipe_reader(input).unwrap();
    let writer = scheduler.pipe_writer(output).unwrap();
    let cancellation = Cancellation::never();
    let mut buffer = [0; 8];
    {
        let mut reading = pin!(reader.read_into(&mut buffer, &cancellation));
        assert!(poll_once(reading.as_mut()).await.is_none());
        drop(writer);
        assert_eq!(reading.await.unwrap(), 0);
    }
    assert_eq!(
        reader.read_into(&mut buffer, &cancellation).await.unwrap(),
        0
    );
    reader.close().await.unwrap();
}

/// Empty transfers check cancellation, then complete immediately without waiting for data or capacity.
pub async fn empty_transfers<S: PipeReading + PipeWriting>(scheduler: &S) {
    let (input, output) = pipe();
    let mut reader = scheduler.pipe_reader(input).unwrap();
    let mut writer = scheduler.pipe_writer(output).unwrap();
    let cancelled = Cancellation::new();
    cancelled.cancel();
    assert_cancelled(reader.read_into(&mut [], &cancelled).await);
    assert_cancelled(reader.read_vectored(&mut [], &cancelled).await);
    assert_cancelled(writer.write(&[], &cancelled).await);
    assert_cancelled(writer.write_vectored(&[], &cancelled).await);
    let cancellation = Cancellation::never();
    let reading = poll_once(pin!(reader.read_into(&mut [], &cancellation))).await;
    assert_eq!(reading.expect("empty reads must not wait").unwrap(), 0);
    let reading = poll_once(pin!(reader.read_vectored(&mut [], &cancellation))).await;
    assert_eq!(reading.expect("empty reads must not wait").unwrap(), 0);
    fill(&mut writer).await;
    let writing = poll_once(pin!(writer.write(&[], &cancellation))).await;
    assert_eq!(writing.expect("empty writes must not wait").unwrap(), 0);
    let writing = poll_once(pin!(writer.write_vectored(&[], &cancellation))).await;
    assert_eq!(writing.expect("empty writes must not wait").unwrap(), 0);
    writer.close().await.unwrap();
    reader.close().await.unwrap();
}

/// Writing after the read end closes reports `BrokenPipe` instead of raising `SIGPIPE`, because Rust executables ignore that signal.
pub async fn broken_pipe<S: PipeWriting>(scheduler: &S) {
    let (input, output) = pipe();
    let probe = io::PipeWriter::from(output.try_clone().unwrap());
    let mut writer = scheduler.pipe_writer(output).unwrap();
    drop(input);
    assert_eq!(broken(probe).kind(), io::ErrorKind::BrokenPipe);
    assert_broken_pipe(writer.write(b"lost", &Cancellation::never()).await);
    assert_broken_pipe(
        writer
            .write_vectored(&[IoSlice::new(b"lost")], &Cancellation::never())
            .await,
    );
    writer.close().await.unwrap();
}

/// Pre-cancellation consumes nothing, and cancelling a pending read or write ends only that wait.
pub async fn cancellation<S: PipeReading + PipeWriting>(scheduler: &S) {
    let (input, output) = pipe();
    let mut reader = scheduler.pipe_reader(input).unwrap();
    let mut writer = scheduler.pipe_writer(output).unwrap();
    let cancelled = Cancellation::new();
    cancelled.cancel();
    let never = Cancellation::never();
    let mut buffer = [0; 8];

    assert_cancelled(writer.write(b"x", &cancelled).await);
    assert_cancelled(
        writer
            .write_vectored(&[IoSlice::new(b"x")], &cancelled)
            .await,
    );
    assert_eq!(writer.write(b"kept", &never).await.unwrap(), 4);
    assert_cancelled(reader.read_into(&mut buffer, &cancelled).await);
    assert_cancelled(
        reader
            .read_vectored(&mut [IoSliceMut::new(&mut buffer)], &cancelled)
            .await,
    );
    assert_eq!(reader.read_into(&mut buffer, &never).await.unwrap(), 4);
    assert_eq!(&buffer[..4], b"kept");

    let pending = Cancellation::new();
    {
        let mut reading = pin!(reader.read_into(&mut buffer, &pending));
        assert!(poll_once(reading.as_mut()).await.is_none());
        pending.cancel();
        assert_cancelled(reading.await);
    }
    let filled = fill(&mut writer).await;
    let pending = Cancellation::new();
    {
        let mut writing = pin!(writer.write(b"x", &pending));
        assert!(poll_once(writing.as_mut()).await.is_none());
        pending.cancel();
        assert_cancelled(writing.await);
    }
    writer.close().await.unwrap();
    assert_eq!(read_to_end(&mut reader).await.len(), filled);
    reader.close().await.unwrap();
}

/// Closing a handle closes its descriptor: the peer observes end of file or a broken pipe, later operations fail with `BrokenPipe`, and repeated close succeeds.
pub async fn close<S: PipeReading + PipeWriting>(scheduler: &S) {
    let never = Cancellation::never();
    let mut buffer = [0; 8];

    let (input, output) = pipe();
    let mut input = io::PipeReader::from(input);
    let mut writer = scheduler.pipe_writer(output).unwrap();
    assert_eq!(writer.write(b"done", &never).await.unwrap(), 4);
    writer.close().await.unwrap();
    let mut observed = Vec::new();
    // The adopted descriptor was the only write end, so a blocking read reaches end of file.
    input.read_to_end(&mut observed).unwrap();
    assert_eq!(observed, b"done");
    assert_broken_pipe(writer.write(b"x", &never).await);
    assert_broken_pipe(writer.write_vectored(&[IoSlice::new(b"x")], &never).await);
    writer.close().await.unwrap();

    let (input, output) = pipe();
    let mut reader = scheduler.pipe_reader(input).unwrap();
    let output = io::PipeWriter::from(output);
    reader.close().await.unwrap();
    assert_eq!(broken(output).kind(), io::ErrorKind::BrokenPipe);
    assert_broken_pipe(reader.read_into(&mut buffer, &never).await);
    assert_broken_pipe(
        reader
            .read_vectored(&mut [IoSliceMut::new(&mut buffer)], &never)
            .await,
    );
    reader.close().await.unwrap();
}
