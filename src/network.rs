//! Application adapter; shared terminal tools stay transport-independent.
use crate::{
    endpoint::{ConnectionState, Endpoint, EndpointError, EndpointId},
    serial::{SerialEndpoint, TransportReader},
    traffic::TrafficBus,
};
use signal_forge_network::{Config, Metadata, State, Transport};
use std::{
    io::{self, Read},
    time::Duration,
};
pub fn is_uri(path: &str) -> bool {
    ["tcp://", "tcp-listen://", "udp://", "udp-listen://"]
        .iter()
        .any(|p| path.starts_with(p))
}
pub fn state(s: State) -> ConnectionState {
    match s {
        State::Connecting => ConnectionState::Connecting,
        State::Listening => ConnectionState::Listening,
        State::Connected => ConnectionState::Connected,
        State::Disconnected => ConnectionState::Disconnected,
        State::Fault(e) => ConnectionState::Fault(e),
    }
}
struct Reader {
    transport: Transport,
    metadata: Option<Metadata>,
}
impl Read for Reader {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        let (n, m) = self.transport.receive(b)?;
        self.metadata = m;
        Ok(n)
    }
}
impl TransportReader for Reader {
    fn set_read_timeout(&mut self, _: Duration) -> Result<(), EndpointError> {
        Ok(())
    }
    fn datagram(&self) -> bool {
        self.transport.is_udp()
    }
    fn metadata(&mut self) -> Option<Metadata> {
        self.metadata.take()
    }
}
pub struct NetworkEndpoint {
    inner: SerialEndpoint,
    transport: Transport,
}
impl NetworkEndpoint {
    pub fn open(uri: &str, bus: TrafficBus) -> Result<Self, EndpointError> {
        Self::open_identified(uri, bus, EndpointId(format!("serial:{uri}")))
    }
    pub fn open_identified(
        uri: &str,
        bus: TrafficBus,
        id: EndpointId,
    ) -> Result<Self, EndpointError> {
        let config = Config::parse(uri).map_err(EndpointError::Io)?;
        let name = config.label();
        let transport = Transport::open(config).map_err(EndpointError::Io)?;
        let inner = SerialEndpoint::from_io_network(
            id,
            name,
            Reader {
                transport: transport.clone(),
                metadata: None,
            },
            io::sink(),
            bus,
            Some(transport.clone()),
        )?;
        Ok(Self { inner, transport })
    }
}
impl Endpoint for NetworkEndpoint {
    fn network(&self) -> Option<Transport> {
        Some(self.transport.clone())
    }
    fn id(&self) -> &EndpointId {
        self.inner.id()
    }
    fn display_name(&self) -> &str {
        self.inner.display_name()
    }
    fn state(&self) -> ConnectionState {
        match self.inner.state() {
            ConnectionState::Fault(e) => ConnectionState::Fault(e),
            ConnectionState::Disconnected => ConnectionState::Disconnected,
            _ => state(self.transport.state()),
        }
    }
    fn send(&self, bytes: Vec<u8>) -> Result<(), EndpointError> {
        if self.state() != ConnectionState::Connected {
            return Err(EndpointError::Disconnected);
        }
        if self.transport.is_udp() && bytes.len() > signal_forge_network::MAX_DATAGRAM {
            return Err(EndpointError::Io("UDP payload exceeds 65507 bytes".into()));
        }
        if self.transport.is_udp() && self.transport.addresses().1.is_none() {
            return Err(EndpointError::Io(
                "Choose an explicit UDP reply target before sending".into(),
            ));
        }
        self.inner.send(bytes)
    }
    fn start_repeat(
        &self,
        b: Vec<u8>,
        s: crate::repeat::RepeatSpec,
    ) -> Result<crate::repeat::RepeatHandle, EndpointError> {
        if self.state() != ConnectionState::Connected {
            return Err(EndpointError::Disconnected);
        }
        if self.transport.is_udp() {
            if b.len() > signal_forge_network::MAX_DATAGRAM {
                return Err(EndpointError::Io("UDP payload exceeds 65507 bytes".into()));
            }
            if self.transport.addresses().1.is_none() {
                return Err(EndpointError::Io(
                    "Choose an explicit UDP reply target before repeating".into(),
                ));
            }
        }
        self.inner.start_repeat(b, s)
    }
    fn send_file(&self, path: &std::path::Path) -> Result<(), EndpointError> {
        self.send_file_mode(
            path,
            crate::file_transfer::FileMode::Raw,
            4096,
            Duration::ZERO,
        )
        .map(|_| ())
    }
    fn send_file_mode(
        &self,
        p: &std::path::Path,
        m: crate::file_transfer::FileMode,
        c: usize,
        d: Duration,
    ) -> Result<crate::file_transfer::FileTransferHandle, EndpointError> {
        if self.state() != ConnectionState::Connected {
            return Err(EndpointError::Disconnected);
        }
        if self.transport.is_udp() && self.transport.addresses().1.is_none() {
            return Err(EndpointError::Io(
                "Choose an explicit UDP reply target before sending a file".into(),
            ));
        }
        if self.transport.is_udp() && c > signal_forge_network::MAX_DATAGRAM {
            return Err(EndpointError::Io(
                "Each file chunk is one UDP datagram; chunk size must be ≤65507".into(),
            ));
        }
        self.inner.send_file_mode(p, m, c, d)
    }
    fn file_send_active(&self) -> bool {
        self.inner.file_send_active()
    }
    fn bridge_port(&self) -> Result<crate::bridge::BridgePort, EndpointError> {
        self.inner.bridge_port()
    }
    fn disconnect(&mut self) {
        self.transport.disconnect();
        self.inner.disconnect();
    }
}
impl Drop for NetworkEndpoint {
    fn drop(&mut self) {
        self.disconnect();
    }
}
