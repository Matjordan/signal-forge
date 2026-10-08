use crate::repeat::{RepeatHandle, RepeatSpec};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EndpointId(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
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

/// Select a transport once; terminal tools and bridges only use Endpoint.
pub fn open(
    settings: &crate::config::SerialSettings,
    bus: crate::traffic::TrafficBus,
) -> Result<Box<dyn Endpoint>, EndpointError> {
    if settings.path.starts_with("ssh://") {
        Ok(Box::new(crate::ssh_serial::SshSerialEndpoint::open(
            settings, bus,
        )?))
    } else {
        Ok(Box::new(crate::serial::SerialEndpoint::open(
            settings, bus,
        )?))
    }
}

/// Transport-independent asynchronous TX interface. RX and successful TX are
/// observed through TrafficBus; physical handles stay in transport workers.
/// Bridges attach to the transport RX path and never subscribe to the monitor bus.
pub trait Endpoint: Send {
    fn id(&self) -> &EndpointId;
    fn display_name(&self) -> &str;
    fn state(&self) -> ConnectionState;
    fn send(&self, bytes: Vec<u8>) -> Result<(), EndpointError>;
    fn send_file(&self, _path: &std::path::Path) -> Result<(), EndpointError> {
        Err(EndpointError::Io(
            "File sending is unavailable for this endpoint".into(),
        ))
    }
    fn file_send_active(&self) -> bool {
        false
    }
    fn start_repeat(&self, bytes: Vec<u8>, spec: RepeatSpec)
        -> Result<RepeatHandle, EndpointError>;
    fn bridge_port(&self) -> Result<crate::bridge::BridgePort, EndpointError>;
    fn disconnect(&mut self);
}

impl<T: Endpoint + ?Sized> Endpoint for Box<T> {
    fn file_send_active(&self) -> bool {
        (**self).file_send_active()
    }
    fn send_file(&self, path: &std::path::Path) -> Result<(), EndpointError> {
        (**self).send_file(path)
    }
    fn id(&self) -> &EndpointId {
        (**self).id()
    }
    fn display_name(&self) -> &str {
        (**self).display_name()
    }
    fn state(&self) -> ConnectionState {
        (**self).state()
    }
    fn send(&self, bytes: Vec<u8>) -> Result<(), EndpointError> {
        (**self).send(bytes)
    }
    fn start_repeat(
        &self,
        bytes: Vec<u8>,
        spec: RepeatSpec,
    ) -> Result<RepeatHandle, EndpointError> {
        (**self).start_repeat(bytes, spec)
    }
    fn bridge_port(&self) -> Result<crate::bridge::BridgePort, EndpointError> {
        (**self).bridge_port()
    }
    fn disconnect(&mut self) {
        (**self).disconnect()
    }
}
