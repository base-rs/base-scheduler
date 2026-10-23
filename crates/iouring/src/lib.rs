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
pub use io_uring_selector::monitor::IoUringMonitorRegistration;
pub use io_uring_selector::multishot::{
    RecvMsgBuffer, RecvMsgMultishot, RecvMsgMultishotOptions, RecvMsgMultishotStatistics,
};

mod io_uring_worker;
pub use io_uring_worker::IoUringWorker;

pub use io_uring_selector::file_monitor::IoUringFileRegistration;

mod io_uring_buffer;
mod io_uring_buffer_pool;
pub use io_uring_buffer::IoUringBuffer;
pub use io_uring_buffer_pool::IoUringBufferPool;

mod io_uring_receiver;
pub use io_uring_receiver::IoUringReceiver;

mod io_uring_recv_message;
pub use io_uring_recv_message::IoUringRecvMessage;
