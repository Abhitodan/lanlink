//! The LANLink host core, shared by the tray app and the console host.
//!
//! [`start`] binds the QUIC endpoint and returns a [`HostHandle`]: a command
//! channel in (open pairing, emergency stop, revoke) and an event channel
//! out (pairing approval requests, session start/end, live statistics).
//! Everything that touches the screen or input runs on dedicated threads;
//! networking runs on the caller's Tokio runtime.

#![cfg(windows)]

pub mod bitrate;
mod connection;
mod host;
mod input_worker;
mod media;
pub mod netpolicy;
pub mod quality;
pub mod storage;

pub use host::{start, HostCommand, HostConfig, HostError, HostEvent, HostHandle};
pub use input_worker::SinkFactory;
