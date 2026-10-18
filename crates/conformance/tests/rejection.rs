// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use futures_lite::future::block_on;
use protocol_scheduler::{
    Blocking, BlockingError, BufferResult, Cancellation, FileOperation, FileRead, FileWrite, IoBuf,
    IoBufMut, TaskError,
};
use protocol_scheduler_conformance::{blocking, file};
use std::{
    future::{Ready, ready},
    panic::{AssertUnwindSafe, catch_unwind},
};

// Intentionally broken implementations establish that the reusable suite rejects violations.
struct IncorrectFile;
impl FileOperation for IncorrectFile {
    type File = ();
}
impl FileRead for IncorrectFile {
    async fn file_read<B: IoBufMut>(&self, _: &(), buffer: B, _: &Cancellation) -> BufferResult<B> {
        (Ok(0), buffer)
    }
}
impl FileWrite for IncorrectFile {
    async fn file_write<B: IoBuf>(&self, _: &(), buffer: B, _: &Cancellation) -> BufferResult<B> {
        (Ok(buffer.as_ref().len()), buffer)
    }
}
#[test]
#[should_panic(expected = "expected cooperative cancellation")]
fn rejects_a_read_that_ignores_cancellation() {
    block_on(file::read(&IncorrectFile, &(), b"data"));
}
#[test]
#[should_panic]
fn rejects_a_write_that_loses_an_expected_error() {
    block_on(file::write_error(&IncorrectFile, &()));
}

#[derive(Clone, Copy)]
enum Fault {
    Eager,
    IgnoreCancellation,
    LosePanic,
}
struct IncorrectBlocking(Fault);
impl Blocking for IncorrectBlocking {
    type Handle<T>
        = Ready<Result<T, TaskError>>
    where
        T: Send + 'static;
    fn spawn_blocking<F, T>(
        &self,
        operation: F,
        cancellation: &Cancellation,
    ) -> impl std::future::Future<Output = Result<Self::Handle<T>, BlockingError>> + Send
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let mode = self.0;
        let mut operation = Some(operation);
        let eager = if matches!(mode, Fault::Eager) {
            Some(
                catch_unwind(AssertUnwindSafe(operation.take().unwrap()))
                    .map_err(TaskError::Panicked),
            )
        } else {
            None
        };
        async move {
            if let Some(result) = eager {
                return Ok(ready(result));
            }
            if cancellation.is_cancelled() && !matches!(mode, Fault::IgnoreCancellation) {
                return Err(BlockingError::Cancelled);
            }
            let result =
                catch_unwind(AssertUnwindSafe(operation.take().unwrap())).map_err(|payload| {
                    if matches!(mode, Fault::LosePanic) {
                        TaskError::Cancelled
                    } else {
                        TaskError::Panicked(payload)
                    }
                });
            Ok(ready(result))
        }
    }
}

#[test]
fn rejects_eager_submission_ignored_cancellation_and_lost_panics() {
    for fault in [Fault::Eager, Fault::IgnoreCancellation, Fault::LosePanic] {
        assert!(
            catch_unwind(AssertUnwindSafe(|| block_on(blocking::lifecycle(
                &IncorrectBlocking(fault)
            ))))
            .is_err()
        );
    }
}
