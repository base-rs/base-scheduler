// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Backend implementation contracts and reusable I/O state.
//!
//! Applications construct scheduler-bound readers and writers through the crate-root capabilities. Adapter authors use this module to compose those handles from owned dispatch, nonblocking borrowed operations, monitors, and receivers. These are public implementation contracts with the same ownership, cancellation, and retirement guarantees; they are not required by the application factory traits.
//!
//! Owned completion operations and borrowed polling operations remain distinct: independently progressing work retains owned storage until retirement, while borrowed storage is accessible only synchronously during each poll.
//!
//! Raw dispatch is intentionally absent from the application namespace:
//! ```compile_fail,E0432
//! use core_scheduler::SocketRead;
//! ```
//! ```compile_fail,E0432
//! use core_scheduler::FileMonitoring;
//! ```

pub use crate::bound_file_reader::BoundFileReader;
pub use crate::bound_file_writer::BoundFileWriter;
pub use crate::bound_socket_packet_reader::BoundSocketPacketReader;
pub use crate::bound_socket_packet_writer::BoundSocketPacketWriter;
pub use crate::bound_socket_stream_reader::BoundSocketStreamReader;
pub use crate::bound_socket_stream_writer::BoundSocketStreamWriter;
pub use crate::file_monitor::FileMonitor;
pub use crate::file_monitor_submission::FileMonitorSubmission;
pub use crate::file_monitoring::FileMonitoring;
pub use crate::file_read::FileRead;
pub use crate::file_read_at::FileReadAt;
pub use crate::file_receiver::FileReceiver;
pub use crate::file_submission::FileSubmission;
pub use crate::file_write::FileWrite;
pub use crate::file_write_all::{file_write_all, file_write_all_submit};
pub use crate::file_write_at::FileWriteAt;
pub use crate::monitor_state::MonitorState;
pub use crate::monitor_storage::MonitorStorage;
pub use crate::socket_messages::SocketMessages;
pub use crate::socket_monitor::SocketMonitor;
pub use crate::socket_monitor_state::SocketMonitorState;
pub use crate::socket_monitoring::SocketMonitoring;
pub use crate::socket_read::SocketRead;
pub use crate::socket_receiver::SocketReceiver;
pub use crate::socket_receiving::SocketReceiving;
pub use crate::socket_recv_msg::SocketRecvMsg;
pub use crate::socket_recv_msg_poll::SocketRecvMsgPoll;
pub use crate::socket_recv_msg_receiver::SocketRecvMsgReceiver;
pub use crate::socket_recv_poll::SocketRecvPoll;
pub use crate::socket_send_msg::SocketSendMsg;
pub use crate::socket_send_msg_poll::SocketSendMsgPoll;
pub use crate::socket_send_poll::SocketSendPoll;
pub use crate::socket_stream_monitor::SocketStreamMonitor;
pub use crate::socket_stream_output::SocketStreamOutput;
pub use crate::socket_stream_outputting::SocketStreamOutputting;
pub use crate::socket_wait::SocketWait;
pub use crate::socket_write::SocketWrite;
pub use crate::socket_write_state::SocketWriteState;
