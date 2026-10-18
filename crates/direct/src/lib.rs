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
#[cfg(unix)]
mod direct_subscription;
mod direct_task;
#[cfg(target_os = "linux")]
mod file_extensions;
mod file_io;
mod resolver;
#[cfg(unix)]
mod signal_service;
mod socket_io;
mod socket_operations;
mod socket_options;
mod socket_retry;
mod system_address_info;
mod system_addresses;
mod system_file_metadata;
mod task_service;
#[cfg(test)]
mod test_support;
mod thread;
mod timer_service;

pub use blocking_task::BlockingTask;
pub use direct_file::DirectFile;
pub use direct_options::DirectOptions;
pub use direct_scheduler::DirectScheduler;
pub use direct_socket::DirectSocket;
#[cfg(unix)]
pub use direct_subscription::DirectSubscription;
pub use direct_task::DirectTask;
pub use system_address_info::SystemAddressInfo;
pub use system_addresses::SystemAddresses;
pub use system_file_metadata::SystemFileMetadata;
