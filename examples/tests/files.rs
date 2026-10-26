// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use base_scheduler::*;
use base_scheduler_examples::files;
use futures_lite::future::block_on;
use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Clone, Default)]
struct File(Arc<Mutex<(Vec<u8>, usize)>>);

struct Files {
    limit: usize,
    closed: AtomicUsize,
    read_calls: AtomicUsize,
    reading: Cancellation,
    writing: Cancellation,
    overlap: bool,
    factory_calls: AtomicUsize,
    fail_factory: usize,
    fail_read: usize,
    fail_close: usize,
}

impl Files {
    fn new(limit: usize, overlap: bool) -> Self {
        Self {
            limit,
            closed: AtomicUsize::new(0),
            read_calls: AtomicUsize::new(0),
            reading: Cancellation::new(),
            writing: Cancellation::new(),
            overlap,
            factory_calls: AtomicUsize::new(0),
            fail_factory: usize::MAX,
            fail_read: usize::MAX,
            fail_close: usize::MAX,
        }
    }
}

impl FileOperation for Files {
    type File = File;
}
impl Files {
    async fn file_read<B: IoBufMut>(
        &self,
        file: &File,
        mut buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        if let Err(error) = cancellation.check() {
            return (Err(error.into()), buffer);
        }
        let call = self.read_calls.fetch_add(1, Ordering::Relaxed);
        if call == self.fail_read {
            return (
                Err(io::Error::from(io::ErrorKind::PermissionDenied).into()),
                buffer,
            );
        }
        if self.overlap && call > 0 {
            self.reading.cancel();
            self.writing.cancelled().await;
        }
        let mut file = file.0.lock().unwrap();
        let count = (file.0.len() - file.1).min(buffer.as_ref().len());
        buffer.as_mut()[..count].copy_from_slice(&file.0[file.1..file.1 + count]);
        file.1 += count;
        (Ok(count), buffer)
    }
}
impl Files {
    async fn file_write<B: IoBuf>(
        &self,
        file: &File,
        buffer: B,
        _: &Cancellation,
    ) -> BufferResult<B> {
        if self.overlap {
            self.writing.cancel();
            self.reading.cancelled().await;
        }
        if self.limit == usize::MAX {
            return (
                Err(io::Error::from(io::ErrorKind::BrokenPipe).into()),
                buffer,
            );
        }
        let count = buffer.as_ref().len().min(self.limit);
        file.0
            .lock()
            .unwrap()
            .0
            .extend_from_slice(&buffer.as_ref()[..count]);
        (Ok(count), buffer)
    }
}
impl Files {
    async fn file_read_at<B: IoBufMut>(
        &self,
        _: &File,
        _: B,
        _: u64,
        _: &Cancellation,
    ) -> BufferResult<B> {
        unreachable!("Recipes use sequential reads")
    }
}
impl Files {
    async fn file_write_at<B: IoBuf>(
        &self,
        _: &File,
        _: B,
        _: u64,
        _: &Cancellation,
    ) -> BufferResult<B> {
        unreachable!("Recipes use sequential writes")
    }
}
struct Reader<'a> {
    scheduler: &'a Files,
    file: File,
}
struct Writer<'a> {
    scheduler: &'a Files,
    file: File,
}
impl Files {
    fn retain_file(&self, file: &File) -> io::Result<File> {
        if self.factory_calls.fetch_add(1, Ordering::Relaxed) == self.fail_factory {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        Ok(file.clone())
    }
    fn retire(&self) -> io::Result<()> {
        if self.closed.fetch_add(1, Ordering::Relaxed) == self.fail_close {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        Ok(())
    }
}
impl FileReader for Reader<'_> {
    async fn read<B: IoBufMut>(
        &mut self,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.scheduler
            .file_read(&self.file, buffer, cancellation)
            .await
    }
    async fn read_at<B: IoBufMut>(
        &mut self,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.scheduler
            .file_read_at(&self.file, buffer, offset, cancellation)
            .await
    }
    async fn close(&mut self) -> io::Result<()> {
        self.scheduler.retire()
    }
}
impl FileWriter for Writer<'_> {
    async fn write<B: IoBuf>(&mut self, buffer: B, cancellation: &Cancellation) -> BufferResult<B> {
        self.scheduler
            .file_write(&self.file, buffer, cancellation)
            .await
    }
    async fn write_at<B: IoBuf>(
        &mut self,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.scheduler
            .file_write_at(&self.file, buffer, offset, cancellation)
            .await
    }
    async fn close(&mut self) -> io::Result<()> {
        self.scheduler.retire()
    }
}

#[test]
fn copy_reports_setup_read_and_cleanup_failures() {
    block_on(async {
        for mode in 0..5 {
            let mut scheduler = Files::new(2, false);
            match mode {
                0 | 1 => scheduler.fail_factory = mode,
                2 => scheduler.fail_read = 1,
                _ => scheduler.fail_close = mode - 3,
            }
            let input = File(Arc::new(Mutex::new((b"data".to_vec(), 0))));
            let output = File::default();
            let result = files::copy(
                &scheduler,
                &input,
                &output,
                [vec![0; 2], vec![0; 2]],
                &Cancellation::never(),
            )
            .await;
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
            if mode >= 2 {
                assert_eq!(scheduler.closed.load(Ordering::Relaxed), 2);
            }
        }
    });
}

#[test]
fn copy_drives_reads_and_writes_together_and_advances_short_writes() {
    block_on(async {
        let scheduler = Files::new(2, true);
        let input = File(Arc::new(Mutex::new((b"elevenbytes".to_vec(), 0))));
        let output = File::default();
        assert_eq!(
            files::copy(
                &scheduler,
                &input,
                &output,
                [vec![0; 4], vec![0; 4]],
                &Cancellation::never()
            )
            .await
            .unwrap(),
            11
        );
        assert_eq!(output.0.lock().unwrap().0, b"elevenbytes");
        assert_eq!(scheduler.closed.load(Ordering::Relaxed), 2);
    });
}

#[test]
fn copy_closes_both_handles_on_cancellation_empty_input_and_write_failure() {
    block_on(async {
        for limit in [0, usize::MAX, 2] {
            let scheduler = Files::new(limit, false);
            let input = File(Arc::new(Mutex::new((b"data".to_vec(), 0))));
            let output = File::default();
            let cancellation = Cancellation::new();
            if limit == 2 {
                cancellation.cancel();
            }
            let error = files::copy(
                &scheduler,
                &input,
                &output,
                [vec![0; 2], vec![0; 2]],
                &cancellation,
            )
            .await
            .unwrap_err();
            let kind = match limit {
                0 => io::ErrorKind::WriteZero,
                2 => io::ErrorKind::Interrupted,
                _ => io::ErrorKind::BrokenPipe,
            };
            assert_eq!(error.kind(), kind);
            assert_eq!(scheduler.closed.load(Ordering::Relaxed), 2);
        }
        let scheduler = Files::new(2, false);
        let file = File::default();
        let never = Cancellation::never();
        assert_eq!(
            files::copy(&scheduler, &file, &file, [vec![0; 2], vec![0; 2]], &never)
                .await
                .unwrap(),
            0
        );
        assert!(
            files::copy(&scheduler, &file, &file, [vec![], vec![0; 2]], &never)
                .await
                .is_err()
        );
        let mut writer = scheduler.file_writer(&file).unwrap();
        assert_eq!(writer.write_all(vec![], &never).await.0.unwrap(), 0);
    });
}

impl base_scheduler::FileReading for Files {
    type Reader<'a> = Reader<'a>;
    fn file_reader(&self, file: &Self::File) -> std::io::Result<Self::Reader<'_>> {
        Ok(Reader {
            scheduler: self,
            file: self.retain_file(file)?,
        })
    }
}

impl base_scheduler::FileWriting for Files {
    type Writer<'a> = Writer<'a>;
    fn file_writer(&self, file: &Self::File) -> std::io::Result<Self::Writer<'_>> {
        Ok(Writer {
            scheduler: self,
            file: self.retain_file(file)?,
        })
    }
}
