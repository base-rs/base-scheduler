// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#![doc = include_str!("../readme.md")]
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(not(any(unix, windows)))]
compile_error!("core-scheduler-selector currently supports Unix and Windows");

mod blocking_pool;
mod blocking_task;
#[cfg(target_os = "linux")]
mod file_extensions;
mod file_io;
mod native_file;
mod native_socket;
#[cfg(unix)]
mod native_subscription;
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
#[cfg(test)]
mod test_support;
#[cfg(unix)]
mod thread;

pub use blocking_task::BlockingTask;
pub use native_file::NativeFile;
pub use native_socket::NativeSocket;
#[cfg(unix)]
pub use native_subscription::NativeSubscription;
pub use system_address_info::SystemAddressInfo;
pub use system_addresses::SystemAddresses;
pub use system_file_metadata::SystemFileMetadata;

mod readiness_selector;
mod selector;
mod selector_options;
pub use readiness_selector::ReadinessSelector;
pub use selector::Selector;
pub use selector_options::SelectorOptions;
