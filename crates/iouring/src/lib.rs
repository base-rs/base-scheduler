// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#![doc = include_str!("../readme.md")]
#![deny(unsafe_op_in_unsafe_fn)]
#![cfg(target_os = "linux")]

mod fallback;
mod io_uring_driver;
mod io_uring_options;
mod io_uring_selector;
pub use io_uring_driver::IoUringDriver;
pub use io_uring_options::IoUringOptions;
pub use io_uring_selector::IoUringSelector;
pub use io_uring_selector::multishot::{
    RecvMsgBuffer, RecvMsgMultishot, RecvMsgMultishotMessage, RecvMsgMultishotOptions,
    RecvMsgMultishotStatistics,
};

mod io_uring_worker;
pub use io_uring_worker::IoUringWorker;
