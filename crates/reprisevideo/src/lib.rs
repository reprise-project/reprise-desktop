//! The screen's PipeWire stream, what plays (a sink's monitor) and the H.264 encoders, shared by
//! the remote desktop server and the screen recorder.

pub mod encode;
pub mod monitor;
pub mod stream;

#[cfg(test)]
pub(crate) mod testlog;
