// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

struct Transfer {
    bytes: Vec<u8>,
    limit: usize,
    fail: bool,
}
impl FileReader for Transfer {
    async fn read<B: IoBufMut>(&mut self, buffer: B, _: &Cancellation) -> BufferResult<B> {
        (
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).into()),
            buffer,
        )
    }
    async fn read_at<B: IoBufMut>(
        &mut self,
        buffer: B,
        _: u64,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.read(buffer, cancellation).await
    }
    async fn close(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl FileWriter for Transfer {
    async fn write<B: IoBuf>(&mut self, buffer: B, _: &Cancellation) -> BufferResult<B> {
        if self.fail {
            return (
                Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).into()),
                buffer,
            );
        }
        let count = self.limit.min(buffer.as_ref().len());
        self.bytes.extend_from_slice(&buffer.as_ref()[..count]);
        (Ok(count), buffer)
    }
    async fn write_at<B: IoBuf>(
        &mut self,
        buffer: B,
        _: u64,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.write(buffer, cancellation).await
    }
    async fn close(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn pooled_input_returns_capacity_after_cancellation_and_read_errors() {
    futures_lite::future::block_on(async {
        let pool = HeapBufferPool::new(1, 4).unwrap();
        let mut reader = Transfer {
            bytes: Vec::new(),
            limit: 0,
            fail: true,
        };
        let cancelled = Cancellation::new();
        cancelled.cancel();
        assert!(matches!(
            read_pooled(&mut reader, &pool, &cancelled).await,
            Err(OperationError::Cancelled(_))
        ));
        assert!(pool.try_acquire().is_some());
        assert_eq!(
            read_pooled(&mut reader, &pool, &Cancellation::never())
                .await
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
        assert!(pool.try_acquire().is_some());
    });
}

#[test]
fn submitted_output_retries_only_the_suffix_and_propagates_errors() {
    futures_lite::future::block_on(async {
        let never = Cancellation::never();
        let mut writer = Transfer {
            bytes: b"ab".to_vec(),
            limit: 2,
            fail: false,
        };
        finish_write(&mut writer, async { (Ok(2), b"abcdef".to_vec()) }, &never)
            .await
            .unwrap();
        assert_eq!(writer.bytes, b"abcdef");
        assert_eq!(
            finish_write(&mut writer, async { (Ok(0), vec![1]) }, &never)
                .await
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::WriteZero
        );
        assert_eq!(writer.bytes, b"abcdef");
        assert_eq!(
            finish_write(
                &mut writer,
                async {
                    (
                        Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).into()),
                        vec![1],
                    )
                },
                &never
            )
            .await
            .unwrap_err()
            .kind(),
            std::io::ErrorKind::PermissionDenied
        );
        writer.fail = true;
        assert_eq!(
            finish_write(&mut writer, async { (Ok(1), vec![1, 2]) }, &never)
                .await
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
    });
}

#[test]
fn pipelines_copy_partial_tails_on_each_backend() {
    #[cfg(target_os = "linux")]
    let backends = ["direct", "tokio", "native", "iouring"];
    #[cfg(not(target_os = "linux"))]
    let backends = ["direct", "tokio", "native"];
    for backend in backends {
        for mode in ["vec-serial", "vec-join", "serial", "join", "submit"] {
            let args = ["files", backend, mode, "4096", "10001", "2"].map(str::to_owned);
            let result = execute(&args);
            assert_eq!(result["bytes"], 10001);
            assert_eq!(result["backend"], backend);
        }
    }
    assert_eq!(execute(&[])["mode"], "join");
}

#[test]
fn empty_input_returns_eof() {
    assert_eq!(
        execute(&["files", "direct", "join", "4096", "0", "1"].map(str::to_owned))["bytes"],
        0
    );
}

#[test]
#[should_panic(expected = "unknown pipeline mode")]
fn rejects_unknown_mode() {
    execute(&["files", "direct", "unknown"].map(str::to_owned));
}

#[test]
#[should_panic(expected = "unknown backend")]
fn rejects_unknown_backend() {
    execute(&["files", "unknown"].map(str::to_owned));
}
