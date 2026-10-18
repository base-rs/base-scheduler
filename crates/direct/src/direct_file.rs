// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::{fs::File, sync::Arc};

/// An owned file for direct scheduler instances. Operations borrow it; explicit close consumes it. It is deliberately not `Clone`.
///
/// The backend privately retains the file while an operation uses it. Dropping an operation future does not establish completion; await the operation before explicitly closing the file. Cancellation-aware work can first receive a cooperative cancellation request; accepted submission-only work retains the descriptor until it finishes. Ordinary handle destruction releases ownership without waiting or reporting close errors.
///
/// ```compile_fail
/// use protocol_scheduler_direct::DirectFile;
/// fn duplicate(file: &DirectFile) -> DirectFile {
///     file.clone() // Cloning this reference cannot produce another owned handle.
/// }
/// ```
#[derive(Debug)]
pub struct DirectFile(pub(crate) Arc<File>);

impl From<File> for DirectFile {
    fn from(file: File) -> Self {
        Self(Arc::new(file))
    }
}

impl AsRef<File> for DirectFile {
    fn as_ref(&self) -> &File {
        &self.0
    }
}

impl DirectFile {
    pub(crate) fn retain(&self) -> Arc<File> {
        Arc::clone(&self.0)
    }

    pub(crate) fn into_file(self) -> File {
        Arc::try_unwrap(self.0)
            .expect("cannot close a file with outstanding operations; cancel and await them first")
    }
}
