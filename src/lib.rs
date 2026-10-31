// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Explicit, runtime-independent scheduler capabilities for protocol libraries.
//!
//! Applications choose an execution environment; libraries ask for the operations they need. This crate defines the interface and runtime-independent support such as [`Cancellation`] and [`HeapBufferPool`]. It contains no executor, reactor, operating-system I/O implementation, or runtime adapter; companion crates such as `base-scheduler-direct`, `base-scheduler-tokio`, and `base-scheduler-native` provide those.
//!
//! # Socket and file I/O
//!
//! Construct a handle from the scheduler once, then pass a [`Cancellation`] to each operation. A handle keeps its scheduler for its whole lifetime, and protocol code can retain it alongside parser state.
//!
//! | Factory | Handle | Operations |
//! | --- | --- | --- |
//! | [`SocketStreamReading`] | [`SocketStreamReader`] | `read` returns an owned chunk; `read_into` and `read_vectored` use borrowed parser storage. |
//! | [`SocketStreamWriting`] | [`SocketStreamWriter`] | `write` and `write_vectored` make borrowed transfers; `submit` accepts owned output that progresses independently, and `flush` waits for all accepted bytes. |
//! | [`SocketPacketReading`] | [`SocketPacketReader`] | `read` returns an owned message with source metadata; borrowed forms fill caller storage and report truncation. |
//! | [`SocketPacketWriting`] | [`SocketPacketWriter`] | Send one packet from borrowed scalar/vector storage or an owned buffer, with an optional destination. |
//! | [`FileReading`] | [`FileReader`] | Owned-buffer `read` at the OS cursor, or `read_at` an explicit offset. |
//! | [`FileWriting`] | [`FileWriter`] | Owned-buffer `write` at the OS cursor, `write_at` an explicit offset, or `write_all`. |
//!
//! Use one handle per sequential execution, with separate readers and writers for concurrent tasks. Borrowed-only readers allocate no receive pool; owned reads initialize a default pool lazily, or use a shared [`BufferPool`] passed to `socket_stream_reader_with_pool` or `socket_packet_reader_with_pool`. Retained chunks can [`freeze`](ReceiveBuffer::freeze) into immutable views that move between tasks. File handles have no private logical position, and file operations return the original buffer owner alongside the [`BufferResult`].
//!
//! Call `close().await` on each handle before dropping it and exclusively closing its resource, flushing successful stream output first. Dropping a handle is not an awaited cleanup barrier.
//!
//! # Other capabilities
//!
//! Protocol libraries require the individual capabilities they use; there is no universal scheduler trait. Socket and file lifecycle operations are separate capabilities, such as [`SocketOpen`], [`SocketBind`], [`SocketListen`], [`SocketConnect`], [`SocketAccept`], [`FileOpen`], [`FileStat`], [`FileSync`], [`SocketClose`], and [`FileClose`].
//!
//! | Capability | Contract |
//! | --- | --- |
//! | [`Clock`] | Monotonic observations, cancellable relative or absolute waits, and operation timeouts. |
//! | [`Timers`] | Experimental reusable timer state for repeated deadline waits. |
//! | [`Spawn`] | Task dispatch with implementation-associated, owning completion handles. |
//! | [`Blocking`] | Bounded admission of owned synchronous work. |
//! | [`ProcessWait`] | Individual child and Unix process-group status waits with cooperative cancellation. |
//! | `PipeReading`, `PipeWriting` | Unix adoption of owned pipe descriptors, such as child process standard streams, as sequential readers and writers. |
//! | [`Signals`] | Notification subscriptions with implementation-associated guards and signal identifiers. |
//! | [`BufferPooling`] | Shareable bounded storage with retained immutable views and cancellable capacity waits. |
//! | Extensions | Typed socket options ([`SocketGetOption`], [`SocketSetOption`]), [`AddressResolution`], [`SocketRegistration`], [`FileAdvice`], and [`FileAllocate`]. |
//!
//! Operation futures use static dispatch and are `Send`; no boxed future or runtime-selection feature is required. Spawned futures and outputs are `Send + 'static`.
//!
//! [`Cancellation`] is a cooperative signal: clones share state, children inherit ancestor cancellation, and dropping a signal does not cancel it. Pre-cancellation prevents submission, successful progress can win a concurrent request, and cancellation does not roll back transferred bytes or cursor changes. A task handle owns its execution: await it to observe completion, or drop it to request destruction after any active poll returns. Keep the scheduler and its drivers alive while accepted work retires.
//!
//! See the [I/O usage guide](https://github.com/base-rs/base-scheduler/blob/main/context/io.md), [interface reference](https://github.com/base-rs/base-scheduler/blob/main/context/interface.md), [design and behavioral contracts](https://github.com/base-rs/base-scheduler/blob/main/context/design.md), and [runnable examples](https://github.com/base-rs/base-scheduler/blob/main/examples/readme.md).
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

mod address_hints;
mod address_info;
mod address_info_collection;
mod address_info_view;
mod address_resolution;
mod address_resolve_error;
mod address_resolve_error_kind;
mod address_service;
mod advice_mode;
mod allocation_mode;
mod blocking;
mod blocking_error;
mod buffer_lease;
mod buffer_pool;
mod buffer_pool_options;
mod buffer_pooling;
mod buffer_result;
mod buffer_sequence_owner;
mod buffer_slice;
mod cancellation;
mod cancelled;
mod clock;
mod defer_cancel;
mod file_advice;
mod file_allocate;
mod file_close;
mod file_metadata;
mod file_open;
mod file_operation;
mod file_options;
mod file_reader;
mod file_reading;
mod file_seek;
mod file_stat;
mod file_sync;
mod file_truncate;
mod file_type;
mod file_writer;
mod file_writing;
mod frozen_buffer;
mod heap_buffer_pool;
mod io_buf;
mod io_buf_mut;
mod io_buf_sequence;
mod io_buf_sequence_mut;
mod io_buffers;
mod io_buffers_mut;
mod notify;
mod operation_error;
mod owned_socket_recv_message;
#[cfg(unix)]
mod pipe_reader;
#[cfg(unix)]
mod pipe_reading;
#[cfg(unix)]
mod pipe_writer;
#[cfg(unix)]
mod pipe_writing;
mod process_wait;
mod process_wait_flags;
mod process_wait_result;
mod receive_buffer;
mod signals;
mod socket_accept;
mod socket_address_view;
mod socket_bind;
mod socket_close;
mod socket_connect;
mod socket_domain;
mod socket_get_option;
mod socket_keepalive;
mod socket_listen;
mod socket_local_address;
mod socket_open;
mod socket_operation;
mod socket_option;
mod socket_options;
mod socket_packet_reader;
mod socket_packet_reading;
mod socket_packet_writer;
mod socket_packet_writing;
mod socket_peer_address;
mod socket_protocol;
mod socket_recv_flags;
mod socket_recv_message;
mod socket_recv_message_mut;
mod socket_recv_result;
mod socket_registration;
mod socket_reuse_address;
mod socket_send_flags;
mod socket_set_option;
mod socket_shutdown;
mod socket_stream_reader;
mod socket_stream_reading;
mod socket_stream_writer;
mod socket_stream_writing;
mod socket_type;
mod spawn;
mod spawn_error;
mod sync_mode;
mod task_error;
mod tcp_no_delay;
mod timeout_error;
mod timers;

pub use address_hints::AddressHints;
pub use address_info::AddressInfo;
pub use address_info_collection::AddressInfoCollection;
pub use address_info_view::AddressInfoView;
pub use address_resolution::AddressResolution;
pub use address_resolve_error::AddressResolveError;
pub use address_resolve_error_kind::AddressResolveErrorKind;
pub use address_service::AddressService;
pub use advice_mode::AdviceMode;
pub use allocation_mode::AllocationMode;
pub use blocking::Blocking;
pub use blocking_error::BlockingError;
pub use buffer_lease::BufferLease;
pub use buffer_pool::BufferPool;
pub use buffer_pool_options::BufferPoolOptions;
pub use buffer_pooling::BufferPooling;
pub use buffer_result::BufferResult;
pub use buffer_slice::BufferSlice;
pub use cancellation::Cancellation;
pub use cancelled::Cancelled;
pub use clock::Clock;
pub use defer_cancel::defer_cancel;
pub use file_advice::FileAdvice;
pub use file_allocate::FileAllocate;
pub use file_close::FileClose;
pub use file_metadata::FileMetadata;
pub use file_open::FileOpen;
pub use file_operation::FileOperation;
pub use file_options::FileOptions;
pub use file_reader::FileReader;
pub use file_reading::FileReading;
pub use file_seek::FileSeek;
pub use file_stat::FileStat;
pub use file_sync::FileSync;
pub use file_truncate::FileTruncate;
pub use file_type::FileType;
pub use file_writer::FileWriter;
pub use file_writing::FileWriting;
pub use frozen_buffer::FrozenBuffer;
pub use heap_buffer_pool::HeapBufferPool;
pub use io_buf::IoBuf;
pub use io_buf_mut::IoBufMut;
pub use io_buf_sequence::IoBufSequence;
pub use io_buf_sequence_mut::IoBufSequenceMut;
pub use io_buffers::IoBuffers;
pub use io_buffers_mut::IoBuffersMut;
pub use notify::Notify;
pub use operation_error::{OperationError, OperationResult};
pub use owned_socket_recv_message::OwnedSocketRecvMessage;
#[cfg(unix)]
pub use pipe_reader::PipeReader;
#[cfg(unix)]
pub use pipe_reading::PipeReading;
#[cfg(unix)]
pub use pipe_writer::PipeWriter;
#[cfg(unix)]
pub use pipe_writing::PipeWriting;
pub use process_wait::ProcessWait;
pub use process_wait_flags::ProcessWaitFlags;
pub use process_wait_result::ProcessWaitResult;
pub use receive_buffer::ReceiveBuffer;
pub use signals::Signals;
pub use socket_accept::SocketAccept;
pub use socket_address_view::SocketAddressView;
pub use socket_bind::SocketBind;
pub use socket_close::SocketClose;
pub use socket_connect::SocketConnect;
pub use socket_domain::SocketDomain;
pub use socket_get_option::SocketGetOption;
pub use socket_keepalive::SocketKeepalive;
pub use socket_listen::SocketListen;
pub use socket_local_address::SocketLocalAddress;
pub use socket_open::SocketOpen;
pub use socket_operation::SocketOperation;
pub use socket_option::SocketOption;
pub use socket_options::SocketOptions;
pub use socket_packet_reader::SocketPacketReader;
pub use socket_packet_reading::SocketPacketReading;
pub use socket_packet_writer::SocketPacketWriter;
pub use socket_packet_writing::SocketPacketWriting;
pub use socket_peer_address::SocketPeerAddress;
pub use socket_protocol::SocketProtocol;
pub use socket_recv_flags::SocketRecvFlags;
pub use socket_recv_message::SocketRecvMessage;
pub use socket_recv_message_mut::SocketRecvMessageMut;
pub use socket_recv_result::SocketRecvResult;
pub use socket_registration::SocketRegistration;
pub use socket_reuse_address::SocketReuseAddress;
pub use socket_send_flags::SocketSendFlags;
pub use socket_set_option::SocketSetOption;
pub use socket_shutdown::SocketShutdown;
pub use socket_stream_reader::SocketStreamReader;
pub use socket_stream_reading::SocketStreamReading;
pub use socket_stream_writer::SocketStreamWriter;
pub use socket_stream_writing::SocketStreamWriting;
pub use socket_type::SocketType;
pub use spawn::Spawn;
pub use spawn_error::SpawnError;
pub use sync_mode::SyncMode;
pub use task_error::TaskError;
pub use tcp_no_delay::TcpNoDelay;
pub use timeout_error::TimeoutError;
pub use timers::Timers;

/// An IP endpoint, compatible with standard-library socket addresses.
pub use std::net::SocketAddr as SocketAddress;

/// Compiles and runs the readme examples as doctests.
#[cfg(doctest)]
#[doc = include_str!("../readme.md")]
pub struct ReadmeDoctests;
