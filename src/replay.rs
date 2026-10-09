//! Validated disk snapshots played by a read-only endpoint. Source files never change.
use crate::{
    bridge::{BridgePort, BridgeWriter},
    endpoint::{ConnectionState, Endpoint, EndpointError, EndpointId},
    inspector::timestamp_ns,
    repeat::{RepeatHandle, RepeatSpec},
    traffic::{Direction, TrafficBus},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplayFormat {
    #[default]
    Raw,
    Hex,
    Capture,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplayStream {
    Both,
    #[default]
    Rx,
    Tx,
}
impl ReplayStream {
    fn includes(self, direction: Direction) -> bool {
        matches!(
            (self, direction),
            (Self::Both, _) | (Self::Rx, Direction::Rx) | (Self::Tx, Direction::Tx)
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ReplayConfig {
    pub path: String,
    pub format: ReplayFormat,
    pub stream: ReplayStream,
    pub chunk: usize,
    pub delay_ms: u64,
}
impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            path: String::new(),
            format: ReplayFormat::Raw,
            stream: ReplayStream::Rx,
            chunk: 4096,
            delay_ms: 10,
        }
    }
}
impl ReplayConfig {
    pub fn uri(&self) -> String {
        format!("replay://{}", serde_json::to_string(self).unwrap())
    }
    pub fn parse(uri: &str) -> Result<Self, String> {
        let config: Self = serde_json::from_str(
            uri.strip_prefix("replay://")
                .ok_or("Not a replay endpoint")?,
        )
        .map_err(|e| e.to_string())?;
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.uri().len() > 4096
            || self.path.is_empty()
            || self.path.contains('\0')
            || self.path.len() > 3000
        {
            return Err("Choose a valid replay file path (≤3000 bytes)".into());
        }
        crate::file_transfer::validate_options(self.chunk, Duration::from_millis(self.delay_ms))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayState {
    Loading,
    Paused,
    Playing,
    Completed,
    Disconnected,
    Failed(String),
}
#[derive(Clone, Debug)]
pub struct ReplayStatus {
    pub state: ReplayState,
    pub events: u64,
    pub bytes: u64,
    pub total_events: u64,
    pub total_bytes: u64,
    pub fast: bool,
    pub warning: Option<String>,
}
struct Shared {
    status: Mutex<ReplayStatus>,
    wake: Condvar,
    stop: AtomicBool,
    epoch: Mutex<u64>,
    step: AtomicBool,
}
#[derive(Clone)]
pub struct ReplayController {
    shared: Arc<Shared>,
    port: BridgePort,
}
impl ReplayController {
    pub fn status(&self) -> ReplayStatus {
        self.shared.status.lock().unwrap().clone()
    }
    pub fn play(&self) {
        let mut status = self.shared.status.lock().unwrap();
        if status.state == ReplayState::Paused {
            status.state = ReplayState::Playing;
        }
        self.shared.wake.notify_all();
    }
    pub fn pause(&self) {
        let mut status = self.shared.status.lock().unwrap();
        if status.state == ReplayState::Playing {
            status.state = ReplayState::Paused;
        }
        self.shared.wake.notify_all();
    }
    pub fn step(&self) {
        let mut status = self.shared.status.lock().unwrap();
        if matches!(status.state, ReplayState::Paused | ReplayState::Playing) {
            status.state = ReplayState::Paused;
            self.shared.step.store(true, Ordering::Release);
        }
        self.shared.wake.notify_all();
    }
    pub fn reset(&self) {
        // Cancels any backpressured bridge write; restarting output is explicit.
        self.port
            .endpoint_closed("Replay reset; restart the bridge before output");
        let mut status = self.shared.status.lock().unwrap();
        if matches!(
            status.state,
            ReplayState::Loading | ReplayState::Disconnected | ReplayState::Failed(_)
        ) {
            return;
        }
        *self.shared.epoch.lock().unwrap() += 1;
        status.events = 0;
        status.bytes = 0;
        status.state = ReplayState::Paused;
        self.shared.step.store(false, Ordering::Release);
        self.shared.wake.notify_all();
    }
    pub fn set_fast(&self, fast: bool) {
        self.shared.status.lock().unwrap().fast = fast;
        self.shared.wake.notify_all();
    }
}
struct SourceWriter {
    shared: Arc<Shared>,
}
impl BridgeWriter for SourceWriter {
    fn state(&self) -> ConnectionState {
        connection(&self.shared.status.lock().unwrap().state)
    }
    fn writable(&self) -> bool {
        false
    }
    fn write(&self, _bytes: &[u8], _cancelled: &AtomicBool) -> Result<bool, EndpointError> {
        Err(EndpointError::Io("Replay is a read-only source".into()))
    }
}
fn connection(state: &ReplayState) -> ConnectionState {
    match state {
        ReplayState::Loading => ConnectionState::Connecting,
        ReplayState::Disconnected => ConnectionState::Disconnected,
        ReplayState::Failed(error) => ConnectionState::Fault(error.clone()),
        _ => ConnectionState::Connected,
    }
}
pub struct ReplayEndpoint {
    id: EndpointId,
    name: String,
    controller: ReplayController,
    worker: Option<JoinHandle<()>>,
}
impl ReplayEndpoint {
    pub fn open(config: ReplayConfig, bus: TrafficBus) -> Result<Self, EndpointError> {
        config.validate().map_err(EndpointError::Io)?;
        let id = EndpointId(format!("serial:{}", config.uri()));
        let name = format!("Replay: {}", config.path);
        let shared = Arc::new(Shared {
            status: Mutex::new(ReplayStatus {
                state: ReplayState::Loading,
                events: 0,
                bytes: 0,
                total_events: 0,
                total_bytes: 0,
                fast: false,
                warning: None,
            }),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
            epoch: Mutex::new(0),
            step: AtomicBool::new(false),
        });
        let port = BridgePort::new(
            id.clone(),
            Arc::new(SourceWriter {
                shared: shared.clone(),
            }),
        );
        let controller = ReplayController {
            shared: shared.clone(),
            port: port.clone(),
        };
        let source = id.clone();
        let worker = thread::Builder::new()
            .name("replay-source".into())
            .spawn(move || {
                let result = (|| -> Result<(), String> {
                    let mut snapshot = Snapshot::load(&config, &shared.stop)?;
                    {
                        let mut status = shared.status.lock().unwrap();
                        if shared.stop.load(Ordering::Acquire) {
                            return Ok(());
                        }
                        status.total_bytes = snapshot.bytes;
                        status.total_events = snapshot.events;
                        status.warning = snapshot.warning.take();
                        status.state = ReplayState::Paused;
                    }
                    let mut epoch = 0;
                    let mut pending = None;
                    let mut previous = None;
                    let mut remaining = Duration::ZERO;
                    while !shared.stop.load(Ordering::Acquire) {
                        let mut status = shared.status.lock().unwrap();
                        let current_epoch = *shared.epoch.lock().unwrap();
                        if epoch != current_epoch {
                            snapshot
                                .file
                                .seek(SeekFrom::Start(0))
                                .map_err(|e| e.to_string())?;
                            epoch = current_epoch;
                            pending = None;
                            previous = None;
                            remaining = Duration::ZERO;
                        }
                        let stepping = shared.step.swap(false, Ordering::AcqRel);
                        if status.state != ReplayState::Playing && !stepping {
                            let _ = shared
                                .wake
                                .wait_timeout(status, Duration::from_millis(20))
                                .unwrap();
                            continue;
                        }
                        if pending.is_none() {
                            pending = snapshot.next()?;
                            remaining = pending
                                .as_ref()
                                .and_then(|unit: &Unit| {
                                    previous.map(|time: i128| {
                                        unit.timestamp.saturating_sub(time).max(0)
                                    })
                                })
                                .map(ns_duration)
                                .transpose()?
                                .unwrap_or_default();
                        }
                        let Some(unit) = pending.as_ref() else {
                            status.state = ReplayState::Completed;
                            continue;
                        };
                        if !status.fast && !stepping && !remaining.is_zero() {
                            let started = Instant::now();
                            drop(
                                shared
                                    .wake
                                    .wait_timeout(status, remaining.min(Duration::from_millis(20)))
                                    .unwrap(),
                            );
                            remaining = remaining.saturating_sub(started.elapsed());
                            continue;
                        }
                        drop(status);
                        port.receive(&unit.bytes);
                        let mut status = shared.status.lock().unwrap();
                        if shared.stop.load(Ordering::Acquire) {
                            break;
                        }
                        if epoch != *shared.epoch.lock().unwrap() {
                            continue;
                        }
                        bus.publish_at(
                            source.clone(),
                            unit.direction,
                            &unit.bytes,
                            unix_time(unit.timestamp)?,
                        );
                        status.events += 1;
                        status.bytes += unit.bytes.len() as u64;
                        previous = Some(unit.timestamp);
                        pending = None;
                        if status.events == status.total_events {
                            status.state = ReplayState::Completed;
                        }
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    if !shared.stop.load(Ordering::Acquire) {
                        shared.status.lock().unwrap().state = ReplayState::Failed(error);
                        port.endpoint_closed("Replay failed");
                    }
                }
            })
            .map_err(|e| EndpointError::Io(e.to_string()))?;
        Ok(Self {
            id,
            name,
            controller,
            worker: Some(worker),
        })
    }
    pub fn controller(&self) -> ReplayController {
        self.controller.clone()
    }
}
impl Endpoint for ReplayEndpoint {
    fn id(&self) -> &EndpointId {
        &self.id
    }
    fn display_name(&self) -> &str {
        &self.name
    }
    fn state(&self) -> ConnectionState {
        connection(&self.controller.status().state)
    }
    fn read_only(&self) -> bool {
        true
    }
    fn playback(&self) -> Option<ReplayController> {
        Some(self.controller())
    }
    fn send(&self, _bytes: Vec<u8>) -> Result<(), EndpointError> {
        Err(EndpointError::Io(
            "Replay is read-only; use playback controls".into(),
        ))
    }
    fn start_repeat(
        &self,
        _bytes: Vec<u8>,
        _spec: RepeatSpec,
    ) -> Result<RepeatHandle, EndpointError> {
        Err(EndpointError::Io("Replay is read-only".into()))
    }
    fn bridge_port(&self) -> Result<BridgePort, EndpointError> {
        if self.state() != ConnectionState::Connected {
            return Err(EndpointError::Disconnected);
        }
        Ok(self.controller.port.clone())
    }
    fn disconnect(&mut self) {
        self.controller.shared.stop.store(true, Ordering::Release);
        self.controller.port.endpoint_closed("Replay disconnected");
        self.controller.shared.wake.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.controller.shared.status.lock().unwrap().state = ReplayState::Disconnected;
    }
}
impl Drop for ReplayEndpoint {
    fn drop(&mut self) {
        self.disconnect();
    }
}
fn ns_duration(ns: i128) -> Result<Duration, String> {
    let ns = u128::try_from(ns).map_err(|_| "Invalid negative duration")?;
    let seconds =
        u64::try_from(ns / 1_000_000_000).map_err(|_| "Timestamp interval is too large")?;
    Ok(Duration::new(seconds, (ns % 1_000_000_000) as u32))
}
fn unix_time(ns: i128) -> Result<SystemTime, String> {
    let duration = ns_duration(ns.checked_abs().ok_or("Timestamp is out of range")?)?;
    (if ns >= 0 {
        UNIX_EPOCH.checked_add(duration)
    } else {
        UNIX_EPOCH.checked_sub(duration)
    })
    .ok_or_else(|| "Timestamp is out of range".into())
}
struct Unit {
    timestamp: i128,
    direction: Direction,
    bytes: Vec<u8>,
}
struct Snapshot {
    file: File,
    events: u64,
    bytes: u64,
    warning: Option<String>,
}
impl Snapshot {
    fn unit(&mut self, timestamp: i128, direction: Direction, bytes: &[u8]) -> Result<(), String> {
        unix_time(timestamp)?;
        if bytes.len() > 65536 {
            return Err("Capture event exceeds 65536 bytes".into());
        }
        self.file
            .write_all(&timestamp.to_le_bytes())
            .and_then(|_| self.file.write_all(&[u8::from(direction == Direction::Tx)]))
            .and_then(|_| self.file.write_all(&(bytes.len() as u32).to_le_bytes()))
            .and_then(|_| self.file.write_all(bytes))
            .map_err(|e| e.to_string())?;
        self.events += 1;
        self.bytes += bytes.len() as u64;
        Ok(())
    }
    fn next(&mut self) -> Result<Option<Unit>, String> {
        let mut time = [0; 16];
        let count = self.file.read(&mut time[..1]).map_err(|e| e.to_string())?;
        if count == 0 {
            return Ok(None);
        }
        self.file
            .read_exact(&mut time[1..])
            .map_err(|e| e.to_string())?;
        let mut direction = [0];
        let mut size = [0; 4];
        self.file
            .read_exact(&mut direction)
            .and_then(|_| self.file.read_exact(&mut size))
            .map_err(|e| e.to_string())?;
        let mut bytes = vec![0; u32::from_le_bytes(size) as usize];
        self.file
            .read_exact(&mut bytes)
            .map_err(|e| e.to_string())?;
        Ok(Some(Unit {
            timestamp: i128::from_le_bytes(time),
            direction: if direction[0] == 0 {
                Direction::Rx
            } else {
                Direction::Tx
            },
            bytes,
        }))
    }
    fn load(config: &ReplayConfig, stop: &AtomicBool) -> Result<Self, String> {
        use std::os::unix::fs::OpenOptionsExt;
        let mut input = OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NONBLOCK)
            .open(&config.path)
            .map_err(|e| e.to_string())?;
        if !input.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("Replay requires a regular file".into());
        }
        let mut output = Self {
            file: tempfile::tempfile().map_err(|e| e.to_string())?,
            events: 0,
            bytes: 0,
            warning: None,
        };
        if config.format == ReplayFormat::Capture {
            output.capture(input, config.stream, stop)?;
        } else {
            let mut hex;
            let reader: &mut dyn Read = if config.format == ReplayFormat::Hex {
                let handle = crate::file_transfer::FileTransferHandle::new();
                hex = crate::file_transfer::prepare(
                    Path::new(&config.path).to_path_buf(),
                    crate::file_transfer::FileMode::Hex,
                    &handle,
                    stop,
                    config.chunk,
                    Duration::ZERO,
                )?;
                &mut hex.file
            } else {
                &mut input
            };
            let mut buffer = vec![0; config.chunk];
            let origin = timestamp_ns(SystemTime::now());
            loop {
                if stop.load(Ordering::Acquire) {
                    return Err("Replay loading cancelled".into());
                }
                let count = reader.read(&mut buffer).map_err(|e| e.to_string())?;
                if count == 0 {
                    break;
                }
                if config.stream != ReplayStream::Tx {
                    output.unit(
                        origin + output.events as i128 * config.delay_ms as i128 * 1_000_000,
                        Direction::Rx,
                        &buffer[..count],
                    )?;
                }
            }
            if config.stream == ReplayStream::Tx {
                output.warning = Some("Raw/Hex RX recordings contain no TX stream".into());
            }
        }
        output
            .file
            .seek(SeekFrom::Start(0))
            .map_err(|e| e.to_string())?;
        Ok(output)
    }
    fn capture(
        &mut self,
        input: File,
        stream: ReplayStream,
        stop: &AtomicBool,
    ) -> Result<(), String> {
        let mut reader = BufReader::new(input);
        let mut line = Vec::new();
        let mut header = false;
        let mut footer = false;
        let mut events = 0u64;
        let mut bytes = 0u64;
        loop {
            if stop.load(Ordering::Acquire) {
                return Err("Replay loading cancelled".into());
            }
            line.clear();
            let count = Read::by_ref(&mut reader)
                .take(1_048_577)
                .read_until(b'\n', &mut line)
                .map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            if count > 1_048_576 {
                return Err("Capture JSON line exceeds 1 MiB".into());
            }
            let value: serde_json::Value =
                serde_json::from_slice(&line).map_err(|e| format!("Invalid capture JSON: {e}"))?;
            match value.get("type").and_then(|v| v.as_str()) {
                Some("header") if !header && events == 0 => {
                    if !matches!(
                        value.get("format").and_then(|v| v.as_str()),
                        Some("signal-forge-capture" | "signal-forge-terminal-capture")
                    ) || value.get("version").and_then(|v| v.as_u64()) != Some(1)
                    {
                        return Err("Unsupported capture format/version".into());
                    }
                    header = true;
                }
                Some("event") if header && !footer => {
                    let data: Vec<u8> = serde_json::from_value(
                        value
                            .get("raw_bytes")
                            .cloned()
                            .ok_or("Missing capture bytes")?,
                    )
                    .map_err(|e| e.to_string())?;
                    if data.len() > 65536 {
                        return Err("Capture event exceeds 65536 bytes".into());
                    }
                    let timestamp: i128 = value
                        .get("timestamp_unix_ns")
                        .and_then(|v| v.as_str())
                        .ok_or("Missing capture timestamp")?
                        .parse()
                        .map_err(|_| "Invalid capture timestamp")?;
                    unix_time(timestamp)?;
                    let direction = match value.get("direction").and_then(|v| v.as_str()) {
                        Some("rx" | "a_to_b") => Direction::Rx,
                        Some("tx" | "b_to_a") => Direction::Tx,
                        _ => return Err("Unsupported capture direction".into()),
                    };
                    events += 1;
                    bytes += data.len() as u64;
                    if stream.includes(direction) {
                        self.unit(timestamp, direction, &data)?;
                    }
                }
                Some("footer") if header && !footer => {
                    if value.get("events").and_then(|v| v.as_u64()) != Some(events)
                        || value.get("bytes").and_then(|v| v.as_u64()) != Some(bytes)
                    {
                        return Err("Capture footer counts do not match its data".into());
                    }
                    footer = true;
                    if value.get("complete").and_then(|v| v.as_bool()) != Some(true)
                        || value
                            .get("dropped_events")
                            .and_then(|v| v.as_u64())
                            .is_some_and(|drops| drops > 0)
                    {
                        self.warning =
                            Some("Capture reports dropped events/incomplete data".into());
                    }
                }
                _ => return Err("Unexpected capture record/order".into()),
            }
        }
        if !header {
            return Err("Missing capture header".into());
        }
        if !footer {
            self.warning = Some("Capture has no footer; recorded data may be incomplete".into());
        }
        Ok(())
    }
}
