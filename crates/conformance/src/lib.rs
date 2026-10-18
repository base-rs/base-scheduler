// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#![doc = include_str!("../readme.md")]

pub mod blocking;
pub mod file;
pub mod socket;
mod socket_io_fixture;
mod socket_io_guard;
pub mod socket_messages;

pub use socket_io_fixture::SocketIoFixture;
pub use socket_io_guard::SocketIoGuard;

mod blocking_queue_tests;
mod blocking_tests;
mod file_test_fixture;
mod file_tests;
mod socket_datagram_tests;
mod socket_io_tests;
mod socket_message_tests;
mod socket_tests;
mod test_cases;

pub use file_test_fixture::FileTestFixture;

// Macro expansions must resolve their support dependency through this crate even when dependencies are renamed by a backend.
#[doc(hidden)]
pub use protocol_scheduler as __protocol_scheduler;
