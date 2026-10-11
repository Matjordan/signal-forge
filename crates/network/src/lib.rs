//! Portable, ordinary-user TCP/UDP transports. No platform-specific device APIs.
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

pub const MAX_DATAGRAM: usize = 65507;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    TcpClient,
    TcpListener,
    UdpClient,
    UdpListener,
}
impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::TcpClient => "TCP client",
            Self::TcpListener => "TCP listener",
            Self::UdpClient => "UDP client",
            Self::UdpListener => "UDP listener",
        }
    }
    pub fn udp(self) -> bool {
        matches!(self, Self::UdpClient | Self::UdpListener)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub mode: Mode,
    pub host: String,
    pub port: u16,
    pub bind: String,
    pub timeout_ms: u64,
    pub reply: Option<SocketAddr>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::TcpClient,
            host: "localhost".into(),
            port: 5000,
            bind: "127.0.0.1:0".into(),
            timeout_ms: 5000,
            reply: None,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=120000).contains(&self.timeout_ms) {
            return Err("Connect timeout must be 1–120000 ms".into());
        }
        let bind = self
            .bind
            .parse::<SocketAddr>()
            .map_err(|_| "Bind address must be an IP and port (IPv6: [::1]:5000)")?;
        if !self.mode.udp() && self.reply.is_some() {
            return Err("Reply targets apply to UDP only".into());
        }
        if self.mode.udp()
            && self
                .reply
                .is_some_and(|peer| peer.port() == 0 || peer.is_ipv4() != bind.is_ipv4())
        {
            return Err(
                "UDP reply target needs a nonzero port and the same IP family as the bind".into(),
            );
        }
        if matches!(self.mode, Mode::TcpClient | Mode::UdpClient)
            && (self.host.trim().is_empty() || self.port == 0)
        {
            return Err("Enter a remote host and a port from 1 to 65535".into());
        }
        Ok(())
    }
    pub fn uri(&self) -> String {
        let scheme = match self.mode {
            Mode::TcpClient => "tcp",
            Mode::TcpListener => "tcp-listen",
            Mode::UdpClient => "udp",
            Mode::UdpListener => "udp-listen",
        };
        let target = if matches!(self.mode, Mode::TcpListener | Mode::UdpListener) {
            self.bind.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        };
        format!(
            "{scheme}://{target}?bind={}&timeout={}{}",
            self.bind,
            self.timeout_ms,
            self.reply
                .map(|p| format!("&reply={p}"))
                .unwrap_or_default()
        )
    }
    pub fn label(&self) -> String {
        let target = if matches!(self.mode, Mode::TcpListener | Mode::UdpListener) {
            self.bind.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        };
        format!("{} · {target}", self.mode.label())
    }
    pub fn parse(uri: &str) -> Result<Self, String> {
        let (scheme, rest) = uri.split_once("://").ok_or("Expected a network URI")?;
        let mode = match scheme {
            "tcp" => Mode::TcpClient,
            "tcp-listen" => Mode::TcpListener,
            "udp" => Mode::UdpClient,
            "udp-listen" => Mode::UdpListener,
            _ => return Err("Unknown network transport".into()),
        };
        let (target, query) = rest.split_once('?').unwrap_or((rest, ""));
        let mut c = Self {
            mode,
            ..Self::default()
        };
        if matches!(mode, Mode::TcpListener | Mode::UdpListener) {
            c.bind = target.into();
        } else {
            let (host, port) = target.rsplit_once(':').ok_or("Expected host:port")?;
            c.host = host.trim_matches(['[', ']']).into();
            c.port = port.parse().map_err(|_| "Invalid remote port")?;
        }
        for part in query.split('&') {
            if let Some((k, v)) = part.split_once('=') {
                match k {
                    "bind" => c.bind = v.into(),
                    "timeout" => c.timeout_ms = v.parse().map_err(|_| "Invalid timeout")?,
                    "reply" => c.reply = Some(v.parse().map_err(|_| "Invalid UDP reply address")?),
                    _ => return Err("Unknown network option".into()),
                }
            }
        }
        c.validate()?;
        Ok(c)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    pub protocol: Mode,
    pub source: SocketAddr,
    pub destination: SocketAddr,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Connecting,
    Listening,
    Connected,
    Disconnected,
    Fault(String),
}
struct Inner {
    state: State,
    tcp: Option<TcpStream>,
    udp: Option<UdpSocket>,
    listener: Option<TcpListener>,
    peer: Option<SocketAddr>,
    local: Option<SocketAddr>,
}
#[derive(Clone)]
pub struct Transport {
    config: Config,
    inner: Arc<Mutex<Inner>>,
    stop: Arc<AtomicBool>,
    writes: Arc<Mutex<()>>,
}
fn err(s: impl std::fmt::Display) -> io::Error {
    io::Error::other(s.to_string())
}
impl Transport {
    pub fn open(config: Config) -> Result<Self, String> {
        config.validate()?;
        let t = Self {
            config,
            inner: Arc::new(Mutex::new(Inner {
                state: State::Connecting,
                tcp: None,
                udp: None,
                listener: None,
                peer: None,
                local: None,
            })),
            stop: Arc::new(AtomicBool::new(false)),
            writes: Arc::new(Mutex::new(())),
        };
        let worker = t.clone();
        thread::Builder::new()
            .name("network-connect".into())
            .spawn(move || {
                if let Err(e) = worker.initialize() {
                    if !worker.stop.load(Ordering::Acquire) {
                        worker.inner.lock().unwrap().state = State::Fault(e.to_string());
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(t)
    }
    fn initialize(&self) -> io::Result<()> {
        let bind: SocketAddr = self.config.bind.parse().map_err(err)?;
        let mut peer = self.config.reply;
        let mut tcp = None;
        let mut udp = None;
        let mut listener = None;
        if self.config.mode == Mode::TcpClient
            || (self.config.mode == Mode::UdpClient && self.config.reply.is_none())
        {
            // OS DNS may block; isolate it so cancellation and the overall deadline stay bounded.
            let (tx, rx) = mpsc::channel();
            let host = self.config.host.clone();
            let port = self.config.port;
            thread::spawn(move || {
                let _ = tx.send(
                    (host.as_str(), port)
                        .to_socket_addrs()
                        .map(|a| a.collect::<Vec<_>>()),
                );
            });
            let deadline = Instant::now() + Duration::from_millis(self.config.timeout_ms);
            let addresses = loop {
                if self.stop.load(Ordering::Acquire) {
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    return Err(err("Host lookup/connect timed out"));
                }
                match rx.recv_timeout(Duration::from_millis(20)) {
                    Ok(result) => break result?,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(e) => return Err(err(e.to_string())),
                }
            };
            let mut last = err("Host has no usable addresses");
            for addr in addresses {
                if self.stop.load(Ordering::Acquire) {
                    return Ok(());
                }
                if self.config.mode == Mode::UdpClient {
                    if addr.is_ipv4() != bind.is_ipv4() {
                        continue;
                    }
                    peer = Some(addr);
                    break;
                }
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                // Preserve the full timeout for high-latency peers. This initializer
                // is detached from the UI and cancelled results are discarded below.
                loop {
                    if self.stop.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        break;
                    }
                    match TcpStream::connect_timeout(&addr, left) {
                        Ok(s) => {
                            s.set_nonblocking(true)?;
                            s.set_nodelay(true)?;
                            tcp = Some(s);
                            peer = Some(addr);
                            break;
                        }
                        Err(e) => {
                            let retry = matches!(
                                e.kind(),
                                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                            );
                            last = e;
                            if !retry {
                                break;
                            }
                        }
                    }
                }
                if tcp.is_some() {
                    break;
                }
            }
            if peer.is_none() {
                return Err(last);
            }
        }
        match self.config.mode {
            Mode::TcpClient => {}
            Mode::TcpListener => {
                let s = TcpListener::bind(bind)?;
                s.set_nonblocking(true)?;
                listener = Some(s);
            }
            Mode::UdpClient | Mode::UdpListener => {
                let s = UdpSocket::bind(bind)?;
                s.set_nonblocking(true)?;
                udp = Some(s);
            }
        }
        let local = if let Some(s) = &tcp {
            Some(s.local_addr()?)
        } else if let Some(s) = &udp {
            Some(s.local_addr()?)
        } else if let Some(s) = &listener {
            Some(s.local_addr()?)
        } else {
            None
        };
        let mut i = self.inner.lock().unwrap();
        if self.stop.load(Ordering::Acquire) {
            return Ok(());
        }
        i.tcp = tcp;
        i.udp = udp;
        i.listener = listener;
        i.peer = peer;
        i.local = local;
        i.state = if self.config.mode == Mode::TcpListener {
            State::Listening
        } else {
            State::Connected
        };
        Ok(())
    }
    pub fn mode(&self) -> Mode {
        self.config.mode
    }
    pub fn is_udp(&self) -> bool {
        self.config.mode.udp()
    }
    pub fn state(&self) -> State {
        self.inner.lock().unwrap().state.clone()
    }
    pub fn addresses(&self) -> (Option<SocketAddr>, Option<SocketAddr>) {
        let i = self.inner.lock().unwrap();
        (i.local, i.peer)
    }
    pub fn set_peer(&self, peer: SocketAddr) -> io::Result<()> {
        if !self.config.mode.udp() {
            return Err(err("Reply targets apply to UDP only"));
        }
        if peer.port() == 0 {
            return Err(err("Reply target port must be 1–65535"));
        }
        let mut i = self.inner.lock().unwrap();
        if i.local.is_some_and(|a| a.is_ipv4() != peer.is_ipv4()) {
            return Err(err(
                "Reply target and bind address use different IP families",
            ));
        }
        i.peer = Some(peer);
        Ok(())
    }
    pub fn receive(&self, buffer: &mut [u8]) -> io::Result<(usize, Option<Metadata>)> {
        let mut i = self.inner.lock().unwrap();
        if self.stop.load(Ordering::Acquire) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if i.tcp.is_none() {
            if let Some(l) = &i.listener {
                match l.accept() {
                    Ok((s, peer)) => {
                        s.set_nonblocking(true)?;
                        s.set_nodelay(true)?;
                        i.local = Some(s.local_addr()?);
                        i.peer = Some(peer);
                        i.tcp = Some(s);
                        i.state = State::Connected;
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e),
                }
            }
        }
        if let Some(s) = &i.udp {
            let (n, source) = s.recv_from(buffer)?;
            return Ok((
                n,
                Some(Metadata {
                    protocol: self.config.mode,
                    source,
                    destination: s.local_addr()?,
                }),
            ));
        }
        if let Some(s) = &mut i.tcp {
            match s.read(buffer) {
                Ok(0) => {
                    i.tcp = None;
                    i.peer = None;
                    if self.config.mode == Mode::TcpListener {
                        i.state = State::Listening;
                    } else {
                        i.state = State::Disconnected;
                        return Err(err("TCP peer closed the connection"));
                    }
                }
                Ok(n) => {
                    return Ok((n, None));
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Err(e),
                Err(e) => {
                    i.tcp = None;
                    i.peer = None;
                    if self.config.mode == Mode::TcpListener {
                        i.state = State::Listening;
                    } else {
                        i.state = State::Disconnected;
                        return Err(e);
                    }
                }
            }
        }
        if let State::Fault(e) = &i.state {
            return Err(err(e.clone()));
        }
        Err(io::ErrorKind::WouldBlock.into())
    }
    pub fn send(&self, bytes: &[u8], cancel: &AtomicBool) -> io::Result<Option<Metadata>> {
        self.send_observed(bytes, cancel, |_, _| {})
    }
    /// Report only bytes accepted by the socket, including partial TCP writes.
    /// UDP invokes the observer exactly once, even for an empty payload.
    pub fn send_observed(
        &self,
        bytes: &[u8],
        cancel: &AtomicBool,
        mut observed: impl FnMut(&[u8], Option<Metadata>),
    ) -> io::Result<Option<Metadata>> {
        let _write = self.writes.lock().unwrap();
        if self.config.mode.udp() {
            if bytes.len() > MAX_DATAGRAM {
                return Err(err("UDP payload exceeds 65507 bytes"));
            }
            let i = self.inner.lock().unwrap();
            let s = i
                .udp
                .as_ref()
                .ok_or_else(|| err("UDP socket is not ready"))?;
            let peer = i
                .peer
                .ok_or_else(|| err("Choose an explicit UDP reply target before sending"))?;
            if self.stop.load(Ordering::Acquire) || cancel.load(Ordering::Acquire) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let n = s.send_to(bytes, peer)?;
            if n != bytes.len() {
                return Err(err("Incomplete UDP datagram"));
            }
            let metadata = Metadata {
                protocol: self.config.mode,
                source: s.local_addr()?,
                destination: peer,
            };
            drop(i);
            observed(bytes, Some(metadata.clone()));
            return Ok(Some(metadata));
        }
        let mut offset = 0;
        let deadline = Instant::now() + Duration::from_secs(5);
        while offset < bytes.len() {
            if self.stop.load(Ordering::Acquire) || cancel.load(Ordering::Acquire) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            if Instant::now() >= deadline {
                self.close_peer();
                return Err(err("TCP write timed out; peer is not reading"));
            }
            let result = {
                let mut i = self.inner.lock().unwrap();
                i.tcp
                    .as_mut()
                    .ok_or_else(|| err("TCP peer is disconnected"))?
                    .write(&bytes[offset..])
            };
            match result {
                Ok(0) => return Err(err("TCP write returned zero")),
                Ok(n) => {
                    observed(&bytes[offset..offset + n], None);
                    offset += n;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(1))
                }
                Err(e) => {
                    let mut i = self.inner.lock().unwrap();
                    i.tcp = None;
                    i.peer = None;
                    i.state = if self.config.mode == Mode::TcpListener {
                        State::Listening
                    } else {
                        State::Disconnected
                    };
                    return Err(e);
                }
            }
        }
        Ok(None)
    }
    fn close_peer(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.tcp = None;
        inner.peer = None;
        inner.state = if self.config.mode == Mode::TcpListener {
            State::Listening
        } else {
            State::Disconnected
        };
    }
    pub fn disconnect(&self) {
        self.stop.store(true, Ordering::Release);
        let mut i = self.inner.lock().unwrap();
        i.tcp = None;
        i.udp = None;
        i.listener = None;
        i.state = State::Disconnected;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Framing {
    Line { strip_ending: bool },
    Fixed { size: usize },
    Idle { gap_ms: u64 },
    Raw,
}
impl Default for Framing {
    fn default() -> Self {
        Self::Line {
            strip_ending: false,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Ending {
    #[default]
    None,
    Lf,
    CrLf,
    Cr,
}
impl Ending {
    pub fn bytes(&self) -> &'static [u8] {
        match self {
            Self::None => b"",
            Self::Lf => b"\n",
            Self::CrLf => b"\r\n",
            Self::Cr => b"\r",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BridgeConfig {
    pub stream_to_udp: Framing,
    pub udp_to_stream: Ending,
}
impl BridgeConfig {
    pub fn validate(&self) -> Result<(), String> {
        match self.stream_to_udp {
            Framing::Fixed { size } if !(1..=MAX_DATAGRAM).contains(&size) => {
                Err("Fixed packet size must be 1–65507 bytes".into())
            }
            Framing::Idle { gap_ms } if !(1..=60000).contains(&gap_ms) => {
                Err("Idle gap must be 1–60000 ms".into())
            }
            _ => Ok(()),
        }
    }
}
pub struct Packetizer {
    pub framing: Framing,
    pending: Vec<u8>,
    last: Option<Instant>,
}
impl Packetizer {
    pub fn new(framing: Framing) -> Self {
        Self {
            framing,
            pending: Vec::new(),
            last: None,
        }
    }
    pub fn push(&mut self, bytes: &[u8], now: Instant) -> Result<Vec<Vec<u8>>, String> {
        let mut out = Vec::new();
        self.push_emitting(bytes, now, |packet| out.push(packet))?;
        Ok(out)
    }
    /// Deliver completed packets before reporting a later malformed/oversized frame.
    pub fn push_emitting(
        &mut self,
        bytes: &[u8],
        now: Instant,
        mut emit: impl FnMut(Vec<u8>),
    ) -> Result<(), String> {
        for packet in self.tick(now) {
            emit(packet);
        }
        if self.framing == Framing::Raw {
            if bytes.len() > MAX_DATAGRAM {
                return Err("Raw read chunk exceeds UDP payload limit".into());
            }
            emit(bytes.to_vec());
            return Ok(());
        }
        for &byte in bytes {
            self.pending.push(byte);
            if self.pending.len() > MAX_DATAGRAM {
                self.pending.clear();
                return Err(
                    "Stream message exceeds UDP payload limit; choose shorter framing".into(),
                );
            }
            let complete = match self.framing {
                Framing::Line { .. } => byte == b'\n',
                Framing::Fixed { size } => self.pending.len() == size,
                _ => false,
            };
            if complete {
                let mut packet = std::mem::take(&mut self.pending);
                if matches!(self.framing, Framing::Line { strip_ending: true }) {
                    packet.pop();
                    if packet.last() == Some(&b'\r') {
                        packet.pop();
                    }
                }
                emit(packet);
            }
        }
        if !bytes.is_empty() {
            self.last = Some(now);
        }
        Ok(())
    }
    pub fn tick(&mut self, now: Instant) -> Vec<Vec<u8>> {
        if let Framing::Idle { gap_ms } = self.framing {
            if !self.pending.is_empty()
                && self.last.is_some_and(|t| {
                    now.saturating_duration_since(t) >= Duration::from_millis(gap_ms)
                })
            {
                self.last = None;
                return vec![std::mem::take(&mut self.pending)];
            }
        }
        Vec::new()
    }
}
