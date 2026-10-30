// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// Portable classification of resolver failures. Native error codes remain available separately and are not errno values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AddressResolveErrorKind {
    /// A temporary failure for which a later retry may succeed, such as `EAI_AGAIN`. Converts to [`io::ErrorKind::WouldBlock`](std::io::ErrorKind::WouldBlock).
    TemporaryFailure,
    /// The name is unknown or has no addresses, such as `EAI_NONAME`. Converts to [`io::ErrorKind::NotFound`](std::io::ErrorKind::NotFound).
    NameNotFound,
    /// The service is not available for the requested socket type, such as `EAI_SERVICE`. Converts to [`io::ErrorKind::NotFound`](std::io::ErrorKind::NotFound).
    ServiceUnavailable,
    /// The hints are invalid or unsupported, such as `EAI_BADFLAGS`, `EAI_FAMILY`, or `EAI_SOCKTYPE`. Converts to [`io::ErrorKind::InvalidInput`](std::io::ErrorKind::InvalidInput).
    InvalidHints,
    /// A non-recoverable resolver failure, such as `EAI_FAIL`. Converts to [`io::ErrorKind::Other`](std::io::ErrorKind::Other).
    PermanentFailure,
    /// The resolver could not allocate memory. Converts to [`io::ErrorKind::OutOfMemory`](std::io::ErrorKind::OutOfMemory).
    OutOfMemory,
    /// An underlying system failure, described by [`AddressResolveError::system_error`](crate::AddressResolveError::system_error). Converts to that error's kind, or [`io::ErrorKind::Other`](std::io::ErrorKind::Other) when it is absent.
    System,
    /// A failure outside the other categories. Converts to [`io::ErrorKind::Other`](std::io::ErrorKind::Other).
    Other,
}
