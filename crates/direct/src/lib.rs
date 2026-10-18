// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#![doc = include_str!("../readme.md")]
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(not(any(unix, windows)))]
compile_error!("protocol-scheduler-direct currently supports Unix and Windows");

mod blocking_pool;
mod blocking_task;
mod direct_file;
mod direct_options;
mod direct_scheduler;
mod direct_socket;
mod file_io;
mod socket_io;
mod socket_operations;
mod socket_retry;
mod system_file_metadata;

pub use blocking_task::BlockingTask;
pub use direct_file::DirectFile;
pub use direct_options::DirectOptions;
pub use direct_scheduler::DirectScheduler;
pub use direct_socket::DirectSocket;
pub use system_file_metadata::SystemFileMetadata;
