// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::backend::*;
use core_scheduler::*;
use futures_lite::future::{block_on, poll_once, zip};
use std::{
    any::Any,
    future::{Future, pending},
    io::{self, Cursor, Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Default)]
struct Files {
    mode: AtomicUsize,
    retained: Mutex<Vec<Box<dyn Any + Send>>>,
    write_limit: AtomicUsize,
}

impl Files {
    fn new(limit: usize) -> Self {
        Self {
            write_limit: AtomicUsize::new(limit),
            ..Self::default()
        }
    }

    fn mode(&self, mode: usize) {
        self.mode.store(mode, Ordering::Relaxed);
    }

    fn submit<B: Send + 'static>(&self, buffer: B) -> Result<Option<B>, (OperationError, B)> {
        match self.mode.load(Ordering::Relaxed) {
            1 => Err((
                io::Error::from(io::ErrorKind::PermissionDenied).into(),
                buffer,
            )),
            2 => {
                self.retained.lock().unwrap().push(Box::new(buffer));
                Ok(None)
            }
            _ => Ok(Some(buffer)),
        }
    }
}

impl FileOperation for Files {
    type File = Arc<Mutex<Cursor<Vec<u8>>>>;
}

impl FileRead for Files {
    async fn file_read<B: IoBufMut>(
        &self,
        file: &Self::File,
        mut buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        if self.mode.load(Ordering::Relaxed) == 2 {
            self.retained.lock().unwrap().push(Box::new(buffer));
            return pending().await;
        }
        let result = cancellation.check().map_err(Into::into).and_then(|_| {
            file.lock()
                .unwrap()
                .read(buffer.as_mut())
                .map_err(Into::into)
        });
        (result, buffer)
    }
}

impl FileWrite for Files {
    async fn file_write<B: IoBuf>(
        &self,
        file: &Self::File,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        if self.mode.load(Ordering::Relaxed) == 2 {
            self.retained.lock().unwrap().push(Box::new(buffer));
            return pending().await;
        }
        let result = cancellation.check().map_err(Into::into).and_then(|_| {
            let length = buffer
                .as_ref()
                .len()
                .min(self.write_limit.load(Ordering::Relaxed));
            file.lock()
                .unwrap()
                .write(&buffer.as_ref()[..length])
                .map_err(Into::into)
        });
        (result, buffer)
    }
}

impl FileReadAt for Files {
    async fn file_read_at<B: IoBufMut>(
        &self,
        file: &Self::File,
        mut buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        if self.mode.load(Ordering::Relaxed) == 2 {
            self.retained.lock().unwrap().push(Box::new(buffer));
            return pending().await;
        }
        let result = cancellation.check().map_err(Into::into).map(|_| {
            let file = file.lock().unwrap();
            let source = &file.get_ref()[offset as usize..];
            let length = source.len().min(buffer.as_ref().len());
            buffer.as_mut()[..length].copy_from_slice(&source[..length]);
            length
        });
        (result, buffer)
    }
}

impl FileWriteAt for Files {
    async fn file_write_at<B: IoBuf>(
        &self,
        file: &Self::File,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        if self.mode.load(Ordering::Relaxed) == 2 {
            self.retained.lock().unwrap().push(Box::new(buffer));
            return pending().await;
        }
        let result = cancellation.check().map_err(Into::into).map(|_| {
            let mut file = file.lock().unwrap();
            let start = offset as usize;
            let length = buffer.as_ref().len();
            let data = file.get_mut();
            data.resize(data.len().max(start + length), 0);
            data[start..start + length].copy_from_slice(buffer.as_ref());
            length
        });
        (result, buffer)
    }
}

impl FileMonitoring for Files {
    type FileRegistration = ();

    fn file_monitor(&self, file: &Self::File) -> io::Result<FileMonitor<Self::File>> {
        if self.mode.load(Ordering::Relaxed) == 3 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(FileMonitor::new(Arc::clone(file), ()))
    }
}

impl FileMonitorSubmission for Files {}

impl FileSubmission for Files {
    async fn file_read_submit<B: IoBufMut>(
        &self,
        file: &Self::File,
        buffer: B,
        cancellation: &Cancellation,
    ) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
        let result = match self.submit(buffer)? {
            Some(buffer) => Some(self.file_read(file, buffer, cancellation).await),
            None => None,
        };
        Ok(async move {
            match result {
                Some(result) => result,
                None => pending().await,
            }
        })
    }

    async fn file_write_submit<B: IoBuf>(
        &self,
        file: &Self::File,
        buffer: B,
        cancellation: &Cancellation,
    ) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
        let result = match self.submit(buffer)? {
            Some(buffer) => Some(self.file_write(file, buffer, cancellation).await),
            None => None,
        };
        Ok(async move {
            match result {
                Some(result) => result,
                None => pending().await,
            }
        })
    }

    async fn file_read_at_submit<B: IoBufMut>(
        &self,
        file: &Self::File,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
        let result = match self.submit(buffer)? {
            Some(buffer) => Some(self.file_read_at(file, buffer, offset, cancellation).await),
            None => None,
        };
        Ok(async move {
            match result {
                Some(result) => result,
                None => pending().await,
            }
        })
    }

    async fn file_write_at_submit<B: IoBuf>(
        &self,
        file: &Self::File,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
        let result = match self.submit(buffer)? {
            Some(buffer) => Some(self.file_write_at(file, buffer, offset, cancellation).await),
            None => None,
        };
        Ok(async move {
            match result {
                Some(result) => result,
                None => pending().await,
            }
        })
    }
}

fn file(data: &[u8]) -> <Files as FileOperation>::File {
    Arc::new(Mutex::new(Cursor::new(data.to_vec())))
}

#[test]
fn bound_files_preserve_shared_cursor_offsets_owners_and_short_writes() {
    block_on(async {
        let scheduler = Files::new(2);
        let file = file(b"abcdef");
        let never = Cancellation::never();
        let mut first = scheduler.file_reader(&file).unwrap();
        let mut second = scheduler.file_reader(&file).unwrap();
        let (count, buffer) = first.read(vec![0; 2], &never).await;
        assert_eq!(count.unwrap(), 2);
        assert_eq!(buffer, b"ab");
        let (count, buffer) = second.read(vec![0; 2], &never).await;
        assert_eq!(count.unwrap(), 2);
        assert_eq!(buffer, b"cd");
        let (count, buffer) = first.read_at(vec![0; 2], 1, &never).await;
        assert_eq!(count.unwrap(), 2);
        assert_eq!(buffer, b"bc");
        assert_eq!(file.lock().unwrap().position(), 4);
        let mut writer = scheduler.file_writer(&file).unwrap();
        let buffer = b"012345".to_vec();
        let pointer = buffer.as_ptr();
        let (count, buffer) = writer.write_all(buffer, &never).await;
        assert_eq!(count.unwrap(), 6);
        assert_eq!(buffer.as_ptr(), pointer);
        assert_eq!(file.lock().unwrap().get_ref(), b"abcd012345");
        assert_eq!(
            writer.write_at(b"xy".to_vec(), 1, &never).await.0.unwrap(),
            2
        );
        assert_eq!(file.lock().unwrap().get_ref(), b"axyd012345");
        assert_eq!(file.lock().unwrap().position(), 10);
        assert_eq!(writer.write_all(Vec::new(), &never).await.0.unwrap(), 0);
        first.close().await.unwrap();
        first.close().await.unwrap();
        second.close().await.unwrap();
        writer.close().await.unwrap();
        writer.close().await.unwrap();
    });
}

#[test]
fn bound_files_reject_construction_cancellation_closed_use_and_zero_progress() {
    block_on(async {
        let scheduler = Files::new(0);
        let file = file(b"abc");
        scheduler.mode(3);
        assert!(scheduler.file_reader(&file).is_err());
        assert!(scheduler.file_writer(&file).is_err());
        scheduler.mode(0);
        let never = Cancellation::never();
        let cancelled = Cancellation::new();
        cancelled.cancel();
        let mut reader = scheduler.file_reader(&file).unwrap();
        let mut writer = scheduler.file_writer(&file).unwrap();
        for result in [
            reader.read(vec![1; 3], &cancelled).await,
            reader.read_at(vec![1; 3], 0, &cancelled).await,
            writer.write(vec![1; 3], &cancelled).await,
            writer.write_at(vec![1; 3], 0, &cancelled).await,
            writer.write_all(vec![1; 3], &cancelled).await,
        ] {
            assert!(matches!(result.0, Err(OperationError::Cancelled(_))));
            assert_eq!(result.1, [1; 3]);
        }
        assert_eq!(
            writer
                .write_all(vec![1], &never)
                .await
                .0
                .unwrap_err()
                .kind(),
            io::ErrorKind::WriteZero
        );
        reader.close().await.unwrap();
        writer.close().await.unwrap();
        for result in [
            reader.read(vec![1], &never).await,
            reader.read_at(vec![1], 0, &never).await,
            writer.write(vec![1], &never).await,
            writer.write_at(vec![1], 0, &never).await,
        ] {
            assert_eq!(result.0.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
            assert_eq!(result.1, [1]);
        }
    });
}

#[test]
fn bound_file_submissions_preserve_admission_rejection_and_retirement() {
    block_on(async {
        let scheduler = Files::new(usize::MAX);
        let file = file(b"abcdef");
        let never = Cancellation::never();
        let cancelled = Cancellation::new();
        cancelled.cancel();
        let mut reader = scheduler.file_reader(&file).unwrap();
        let mut writer = scheduler.file_writer(&file).unwrap();
        for mode in [0, 1, 2] {
            scheduler.mode(mode);
            for operation in 0..4 {
                let token = if mode == 0 { &cancelled } else { &never };
                let buffer = vec![1; 2];
                let error = match operation {
                    0 => reader.read_submit(buffer, token).await.err(),
                    1 => writer.write_submit(buffer, token).await.err(),
                    2 => reader.read_at_submit(buffer, 0, token).await.err(),
                    _ => writer.write_at_submit(buffer, 0, token).await.err(),
                };
                if mode == 2 {
                    assert!(error.is_none());
                    let (result, owner) = if operation % 2 == 0 {
                        reader.read(vec![42], &never).await
                    } else {
                        writer.write(vec![42], &never).await
                    };
                    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::WouldBlock);
                    assert_eq!(owner, [42]);
                    let mut close = if operation % 2 == 0 {
                        // Test close waits for the backend owner, independently of the dropped completion.
                        Box::pin(reader.close())
                            as std::pin::Pin<Box<dyn Future<Output = io::Result<()>> + Send>>
                    } else {
                        Box::pin(writer.close())
                    };
                    assert!(poll_once(close.as_mut()).await.is_none());
                    scheduler.retained.lock().unwrap().clear();
                    close.await.unwrap();
                    reader = scheduler.file_reader(&file).unwrap();
                    writer = scheduler.file_writer(&file).unwrap();
                } else {
                    let (error, owner) = error.unwrap();
                    assert_eq!(owner, [1; 2]);
                    if mode == 0 {
                        assert!(matches!(error, OperationError::Cancelled(_)));
                    } else {
                        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
                    }
                }
            }
        }
        scheduler.mode(0);
        assert_eq!(
            reader
                .read_submit(vec![0; 2], &never)
                .await
                .unwrap()
                .await
                .0
                .unwrap(),
            2
        );
        assert_eq!(
            reader
                .read_at_submit(vec![0; 2], 0, &never)
                .await
                .unwrap()
                .await
                .0
                .unwrap(),
            2
        );
        assert_eq!(
            writer
                .write_submit(vec![42; 2], &never)
                .await
                .unwrap()
                .await
                .0
                .unwrap(),
            2
        );
        assert_eq!(
            writer
                .write_at_submit(vec![42; 2], 0, &never)
                .await
                .unwrap()
                .await
                .0
                .unwrap(),
            2
        );
        reader.close().await.unwrap();
        writer.close().await.unwrap();
    });
}

#[test]
fn separate_bound_handles_drive_read_write_overlap() {
    block_on(async {
        let scheduler = Files::new(usize::MAX);
        let input = file(b"abc");
        let output = file(b"");
        let mut reader = scheduler.file_reader(&input).unwrap();
        let mut writer = scheduler.file_writer(&output).unwrap();
        let never = Cancellation::never();
        let (read, written) = zip(
            reader.read(vec![0; 3], &never),
            writer.write_all(b"def".to_vec(), &never),
        )
        .await;
        assert_eq!(read.0.unwrap(), 3);
        assert_eq!(read.1, b"abc");
        assert_eq!(written.0.unwrap(), 3);
        assert_eq!(output.lock().unwrap().get_ref(), b"def");
    });
}

#[test]
fn abandoned_ordinary_file_operations_block_submission_until_retirement() {
    block_on(async {
        let scheduler = Files::new(usize::MAX);
        scheduler.mode(2);
        let file = file(b"abcdef");
        let never = Cancellation::never();
        macro_rules! check {
            ($factory:ident, $ordinary:ident, $submit:ident $(, $offset:expr)?) => {{
                let mut handle = scheduler.$factory(&file).unwrap();
                {
                    let mut operation = std::pin::pin!(handle.$ordinary(vec![42], $($offset,)? &never));
                    assert!(poll_once(operation.as_mut()).await.is_none());
                }
                let (error, buffer) = handle.$submit(vec![7], $($offset,)? &never).await.err().unwrap();
                assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
                assert_eq!(buffer, [7]);
                let mut close = std::pin::pin!(handle.close());
                assert!(poll_once(close.as_mut()).await.is_none());
                scheduler.retained.lock().unwrap().clear();
                close.await.unwrap();
            }};
        }
        check!(file_reader, read, read_submit);
        check!(file_writer, write, write_submit);
        check!(file_reader, read_at, read_at_submit, 0);
        check!(file_writer, write_at, write_at_submit, 0);
    });
}

impl FileReading for Files {
    type Reader<'scheduler> = BoundFileReader<'scheduler, Self>;
    fn file_reader(&self, file: &Self::File) -> io::Result<Self::Reader<'_>> {
        BoundFileReader::new(self, file)
    }
}

impl FileWriting for Files {
    type Writer<'scheduler> = BoundFileWriter<'scheduler, Self>;
    fn file_writer(&self, file: &Self::File) -> io::Result<Self::Writer<'_>> {
        BoundFileWriter::new(self, file)
    }
}
