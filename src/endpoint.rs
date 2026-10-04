use crate::repeat::{RepeatHandle, RepeatSpec};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EndpointId(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Connected,
    Disconnected,
    Fault(String),
}

#[derive(Debug, thiserror::Error)]
pub enum EndpointError {
    #[error("endpoint is disconnected")]
    Disconnected,
    #[error("send queue is full; retry after pending writes finish")]
    QueueFull,
    #[error("I/O error: {0}")]
    Io(String),
}

/// Transport-independent asynchronous TX interface. RX and successful TX are
/// observed through TrafficBus; physical handles stay in transport workers.
/// Future bridges should subscribe to a dedicated lossless RX route in workers.
pub trait Endpoint: Send {
    fn id(&self) -> &EndpointId;
    fn display_name(&self) -> &str;
    fn state(&self) -> ConnectionState;
    fn send(&self, bytes: Vec<u8>) -> Result<(), EndpointError>;
    fn start_repeat(&self, bytes: Vec<u8>, spec: RepeatSpec)
        -> Result<RepeatHandle, EndpointError>;
    fn bridge_port(&self) -> Result<crate::bridge::BridgePort, EndpointError>;
    fn disconnect(&mut self);
}
