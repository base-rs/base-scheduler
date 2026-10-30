// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// Portable regular-file opening options. Fields are visible to alternative implementations, including simulated filesystems. Append mode is omitted: positioned writes must target the supplied offset.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FileOptions {
    /// Open with read access.
    pub read: bool,
    /// Open with write access.
    pub write: bool,
    /// Create the file if it does not already exist. Requires `write`.
    pub create: bool,
    /// Create a new file, failing if one already exists at the path. Requires `write`, and takes precedence over `create` and `truncate`.
    pub create_new: bool,
    /// Truncate an existing file to zero length when opening it. Requires `write`.
    pub truncate: bool,
}

impl FileOptions {
    /// Open an existing file for reading only.
    pub const READ: Self = Self {
        read: true,
        write: false,
        create: false,
        create_new: false,
        truncate: false,
    };
    /// Open a file for writing only, creating it if missing and truncating it otherwise.
    pub const CREATE: Self = Self {
        read: false,
        write: true,
        create: true,
        create_new: false,
        truncate: true,
    };
}
