// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#![doc = include_str!("../readme.md")]
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(not(any(unix, windows)))]
compile_error!("core-scheduler-tokio currently supports Unix and Windows");

mod blocking_pool;
mod blocking_task;
mod clock;
#[cfg(target_os = "linux")]
mod file_extensions;
mod file_io;
mod panic;
mod resolver;
#[cfg(unix)]
mod signal_service;
mod socket_io;
mod socket_operations;
mod socket_options;
mod system_address_info;
mod system_addresses;
mod system_file_metadata;
mod task_service;
#[cfg(test)]
mod test_support;
#[cfg(unix)]
mod thread;
mod tokio_file;
mod tokio_options;
mod tokio_scheduler;
mod tokio_socket;
#[cfg(unix)]
mod tokio_subscription;
mod tokio_task;

pub use blocking_task::BlockingTask;
pub use system_address_info::SystemAddressInfo;
pub use system_addresses::SystemAddresses;
pub use system_file_metadata::SystemFileMetadata;
pub use tokio_file::TokioFile;
pub use tokio_options::TokioOptions;
pub use tokio_scheduler::TokioScheduler;
pub use tokio_socket::TokioSocket;
#[cfg(unix)]
pub use tokio_subscription::TokioSubscription;
pub use tokio_task::TokioTask;

mod tokio_timer;
pub use tokio_timer::TokioTimer;

mod file_monitoring;

mod file_submission;

mod socket_receiving;
