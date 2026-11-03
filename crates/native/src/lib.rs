// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#![doc = include_str!("../readme.md")]
#![deny(unsafe_op_in_unsafe_fn)]

mod monitor;
mod native_executor_options;
mod native_options;
mod native_scheduler;
mod native_task;
mod panic;
#[cfg(feature = "diagnostics")]
mod rescue_statistics;
mod selector_operations;
mod spin_lock;
mod task_service;
#[cfg(test)]
mod test_support;
mod thread;
mod worker;
mod worker_timers;

#[cfg(unix)]
pub use base_scheduler_selector::NativeSubscription;
pub use base_scheduler_selector::{
    BlockingTask, NativeFile, NativeSocket, ReadinessSelector, Selector, SelectorOptions,
    SystemAddressInfo, SystemAddresses, SystemFileMetadata,
};
pub use native_executor_options::NativeExecutorOptions;
pub use native_options::NativeOptions;
pub use native_scheduler::NativeScheduler;
pub use native_task::NativeTask;
#[cfg(feature = "diagnostics")]
pub use rescue_statistics::RescueStatistics;

pub use worker_timers::NativeTimer;
