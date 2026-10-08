//! OpenSSH-backed serial streams. Authentication and trust stay in OpenSSH.
use crate::{
    bridge::BridgePort,
    config::SerialSettings,
    endpoint::{ConnectionState, Endpoint, EndpointError, EndpointId},
    repeat::{RepeatHandle, RepeatSpec},
    serial::SerialEndpoint,
    traffic::TrafficBus,
};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    os::fd::AsRawFd,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshHost {
    pub host: String,
    pub username: Option<String>,
    pub port: Option<u16>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSshHost {
    pub name: String,
    pub connection: SshHost,
}
impl SshHost {
    pub fn parse(path: &str) -> Result<(Self, String), EndpointError> {
        let value = path
            .strip_prefix("ssh://")
            .ok_or_else(|| error("Expected ssh://host/device"))?;
        let (authority, device) = value
            .split_once('/')
            .ok_or_else(|| error("Remote device path is required"))?;
        let (username, address) = match authority.split_once('@') {
            Some((user, host)) => (Some(user.to_string()), host),
            None => (None, authority),
        };
        let (host, port) = match address.split_once(':') {
            Some((host, port)) => (
                host.to_string(),
                Some(port.parse::<u16>().map_err(|_| error("Invalid SSH port"))?),
            ),
            None => (address.to_string(), None),
        };
        let result = Self {
            host,
            username,
            port,
        };
        result.validate()?;
        let device = format!("/{device}");
        if !device.starts_with("/dev/") || device.contains(['\0', '\n', '\r']) {
            return Err(error("Remote serial device must be an absolute /dev/ path"));
        }
        Ok((result, device))
    }
    pub fn validate(&self) -> Result<(), EndpointError> {
        let valid = |s: &str| {
            !s.is_empty()
                && !s.starts_with('-')
                && s.len() <= 255
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        };
        if !valid(&self.host)
            || self.username.as_ref().is_some_and(|s| !valid(s))
            || self.port == Some(0)
        {
            return Err(error("Invalid SSH host, username or port"));
        }
        Ok(())
    }
    pub fn uri(&self, device: &str) -> String {
        format!(
            "ssh://{}{}{}{}",
            self.username
                .as_ref()
                .map(|s| format!("{s}@"))
                .unwrap_or_default(),
            self.host,
            self.port.map(|p| format!(":{p}")).unwrap_or_default(),
            device
        )
    }
    fn command(&self, argument: &str) -> Result<Command, EndpointError> {
        self.validate()?;
        let mut command = Command::new("ssh");
        command.args([
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "ServerAliveInterval=5",
            "-o",
            "ServerAliveCountMax=2",
            "-o",
            "ClearAllForwardings=yes",
            "-o",
            "PermitLocalCommand=no",
            "-o",
            "RequestTTY=no",
            "-o",
            "RemoteCommand=none",
        ]);
        if let Some(user) = &self.username {
            command.args(["-l", user]);
        }
        if let Some(port) = self.port {
            command.args(["-p", &port.to_string()]);
        }
        command.arg(&self.host).arg(format!(
            "python3 -c {} {}",
            quote(include_str!("remote_serial.py")),
            quote(argument)
        ));
        Ok(command)
    }
}
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
fn error(message: impl Into<String>) -> EndpointError {
    EndpointError::Io(message.into())
}
fn nonblocking(fd: &impl AsRawFd) -> Result<(), EndpointError> {
    let raw = fd.as_raw_fd();
    // SAFETY: valid borrowed descriptors, preserving their existing flags.
    unsafe {
        let flags = nix::libc::fcntl(raw, nix::libc::F_GETFL);
        if flags < 0 || nix::libc::fcntl(raw, nix::libc::F_SETFL, flags | nix::libc::O_NONBLOCK) < 0
        {
            return Err(error(std::io::Error::last_os_error().to_string()));
        }
    }
    Ok(())
}

pub struct SshSerialEndpoint {
    inner: SerialEndpoint,
    state: Arc<std::sync::Mutex<ConnectionState>>,
    stop: Arc<AtomicBool>,
    supervisor: Option<JoinHandle<()>>,
}
impl SshSerialEndpoint {
    pub fn open(settings: &SerialSettings, bus: TrafficBus) -> Result<Self, EndpointError> {
        settings.validate().map_err(error)?;
        let (host, device) = SshHost::parse(&settings.path)?;
        let mut remote = settings.clone();
        remote.path = device;
        let argument = serde_json::to_string(&remote).map_err(|e| error(e.to_string()))?;
        let mut child = host
            .command(&argument)?
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| error(format!("SSH launch: {e}. Install openssh-client.")))?;
        let prepare = (|| {
            let stdin = child.stdin.take().unwrap();
            let stdout = child.stdout.take().unwrap();
            let stderr = child.stderr.take().unwrap();
            nonblocking(&stdin)?;
            nonblocking(&stdout)?;
            nonblocking(&stderr)?;
            let inner = SerialEndpoint::from_io(
                EndpointId(format!("serial:{}", settings.path)),
                settings.path.clone(),
                stdout,
                stdin,
                bus,
            )?;
            Ok::<_, EndpointError>((inner, stderr))
        })();
        let (inner, mut stderr) = match prepare {
            Ok(value) => value,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let transport_state = inner.shared_state();
        let state = Arc::new(std::sync::Mutex::new(ConnectionState::Connecting));
        let visible_state = state.clone();
        *transport_state.lock().unwrap() = ConnectionState::Connecting;
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let supervisor = thread::spawn(move || {
            let start = Instant::now();
            let mut diagnostics = String::new();
            let mut ready = false;
            let mut buffer = [0; 4096];
            while !worker_stop.load(Ordering::Acquire) {
                match stderr.read(&mut buffer) {
                    Ok(n) if n > 0 => {
                        if diagnostics.len() < 16384 {
                            diagnostics.push_str(&String::from_utf8_lossy(&buffer[..n]));
                        }
                        if !ready && diagnostics.lines().any(|l| l == "SIGNAL_FORGE_READY") {
                            let mut current = transport_state.lock().unwrap();
                            if *current == ConnectionState::Connecting {
                                *current = ConnectionState::Connected;
                                *visible_state.lock().unwrap() = ConnectionState::Connected;
                                log::info!("SSH remote serial connected");
                            }
                            ready = true;
                        }
                    }
                    Err(e)
                        if !matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                        ) =>
                    {
                        diagnostics.push_str(&format!("SSH diagnostics: {e}"));
                        break;
                    }
                    _ => {}
                }
                match child.try_wait() {
                    Ok(Some(status)) => {
                        diagnostics.push_str(&format!("\nSSH / remote helper exited: {status}"));
                        break;
                    }
                    Err(e) => {
                        diagnostics.push_str(&format!("SSH process: {e}"));
                        break;
                    }
                    _ => {}
                }
                if !ready && start.elapsed() > Duration::from_secs(15) {
                    diagnostics.push_str("SSH / remote helper readiness timed out");
                    break;
                }
                if matches!(*transport_state.lock().unwrap(), ConnectionState::Fault(_)) {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            let _ = child.kill();
            let _ = child.wait();
            let mut remaining = Vec::new();
            let _ = stderr.take(16384).read_to_end(&mut remaining);
            diagnostics.push_str(&String::from_utf8_lossy(&remaining));
            if !worker_stop.load(Ordering::Acquire) {
                let diagnostics = diagnostics.replace("SIGNAL_FORGE_READY", "");
                let diagnostics = if diagnostics.trim().is_empty() {
                    format!(
                        "SSH session lost or remote helper stopped: {:?}",
                        *transport_state.lock().unwrap()
                    )
                } else {
                    diagnostics
                };
                *visible_state.lock().unwrap() =
                    ConnectionState::Fault(format!("SSH remote serial: {}", diagnostics.trim()));
            }
        });
        Ok(Self {
            inner,
            state,
            stop,
            supervisor: Some(supervisor),
        })
    }
}
impl Endpoint for SshSerialEndpoint {
    fn file_send_active(&self) -> bool {
        self.inner.file_send_active()
    }
    fn send_file(&self, path: &std::path::Path) -> Result<(), EndpointError> {
        if self.state() != ConnectionState::Connected {
            return Err(EndpointError::Disconnected);
        }
        self.inner.send_file(path)
    }
    fn id(&self) -> &EndpointId {
        self.inner.id()
    }
    fn display_name(&self) -> &str {
        self.inner.display_name()
    }
    fn state(&self) -> ConnectionState {
        self.state.lock().unwrap().clone()
    }
    fn send(&self, bytes: Vec<u8>) -> Result<(), EndpointError> {
        if self.state() != ConnectionState::Connected {
            return Err(EndpointError::Disconnected);
        }
        self.inner.send(bytes)
    }
    fn start_repeat(
        &self,
        bytes: Vec<u8>,
        spec: RepeatSpec,
    ) -> Result<RepeatHandle, EndpointError> {
        if self.state() != ConnectionState::Connected {
            return Err(EndpointError::Disconnected);
        }
        self.inner.start_repeat(bytes, spec)
    }
    fn bridge_port(&self) -> Result<BridgePort, EndpointError> {
        if self.state() != ConnectionState::Connected {
            return Err(EndpointError::Disconnected);
        }
        self.inner.bridge_port()
    }
    fn disconnect(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.supervisor.take() {
            let _ = worker.join();
        }
        self.inner.disconnect();
        *self.state.lock().unwrap() = ConnectionState::Disconnected;
    }
}
impl Drop for SshSerialEndpoint {
    fn drop(&mut self) {
        self.disconnect();
    }
}

pub fn discover(host: &SshHost) -> Result<Vec<String>, EndpointError> {
    discover_cancellable(host, &AtomicBool::new(false))
}

pub struct Discovery {
    receiver: std::sync::mpsc::Receiver<Result<Vec<String>, String>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Discovery {
    pub fn start(host: SshHost) -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = stop.clone();
        let worker = thread::spawn(move || {
            let _ = sender.send(discover_cancellable(&host, &cancel).map_err(|e| e.to_string()));
        });
        Self {
            receiver,
            stop,
            worker: Some(worker),
        }
    }
    pub fn try_result(&self) -> Option<Result<Vec<String>, String>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Some(Err("SSH discovery worker stopped".into()))
            }
        }
    }
}
impl Drop for Discovery {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn discover_cancellable(host: &SshHost, stop: &AtomicBool) -> Result<Vec<String>, EndpointError> {
    let mut child = host
        .command("discover")?
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| error(format!("SSH discovery: {e}")))?;
    let result = (|| {
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        nonblocking(&stdout)?;
        nonblocking(&stderr)?;
        let mut output = Vec::new();
        let mut diagnostic = Vec::new();
        let mut buffer = [0; 4096];
        let start = Instant::now();
        loop {
            if stop.load(Ordering::Acquire) {
                return Err(error("SSH discovery cancelled"));
            }
            for (reader, bytes) in [
                (&mut stdout as &mut dyn Read, &mut output),
                (&mut stderr as &mut dyn Read, &mut diagnostic),
            ] {
                match reader.read(&mut buffer) {
                    Ok(n) => bytes.extend_from_slice(&buffer[..n]),
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                        ) => {}
                    Err(e) => return Err(error(e.to_string())),
                }
                if bytes.len() > 65536 {
                    return Err(error("SSH discovery output exceeded limit"));
                }
            }
            if let Some(status) = child.try_wait().map_err(|e| error(e.to_string()))? {
                // Drain the final pipe contents after process exit.
                let _ = stdout.read_to_end(&mut output);
                let _ = stderr.read_to_end(&mut diagnostic);
                if !status.success() {
                    return Err(error(format!(
                        "SSH discovery: {}",
                        String::from_utf8_lossy(&diagnostic)
                    )));
                }
                let paths: Vec<String> = serde_json::from_slice(&output)
                    .map_err(|e| error(format!("Remote discovery response: {e}")))?;
                if paths
                    .iter()
                    .any(|p| !p.starts_with("/dev/") || p.contains(['\0', '\n', '\r']))
                {
                    return Err(error("Invalid remote device path"));
                }
                return Ok(paths);
            }
            if start.elapsed() > Duration::from_secs(15) {
                return Err(error("SSH discovery timed out"));
            }
            thread::sleep(Duration::from_millis(10));
        }
    })();
    let _ = child.kill();
    let _ = child.wait();
    result
}
