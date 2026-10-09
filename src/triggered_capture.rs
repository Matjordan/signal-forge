//! Bounded rolling capture evaluated on a worker, independent of presentation.
use crate::{
    endpoint::EndpointId,
    inspector::timestamp_ns,
    traffic::{Direction, TrafficBus, TrafficEvent},
    traffic_analysis::{Matcher, Pattern},
};
use std::{
    collections::VecDeque,
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime},
};
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Trigger {
    #[default]
    Manual,
    Pattern(Pattern),
    RxIdle(Duration),
}
#[derive(Clone, Debug)]
pub struct CaptureOptions {
    pub trigger: Trigger,
    pub pre_bytes: usize,
    pub pre_duration: Duration,
    pub post_bytes: usize,
    pub post_duration: Duration,
}
impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            trigger: Trigger::Manual,
            pre_bytes: 1024 * 1024,
            pre_duration: Duration::from_secs(5),
            post_bytes: 65536,
            post_duration: Duration::from_secs(1),
        }
    }
}
impl CaptureOptions {
    pub fn validate(&self) -> Result<(), String> {
        if self.pre_bytes > 16 * 1024 * 1024
            || self.pre_duration > Duration::from_secs(3600)
            || self.post_bytes > 1024 * 1024 * 1024
            || self.post_duration > Duration::from_secs(3600)
        {
            return Err("Capture limits: pre ≤16 MiB, post ≤1 GiB, windows ≤1 hour".into());
        }
        match &self.trigger {
            Trigger::Pattern(pattern) => {
                if pattern.compile()?.is_none() {
                    return Err("Enter a trigger pattern".into());
                }
            }
            Trigger::RxIdle(idle) if idle.is_zero() || *idle > Duration::from_secs(3600) => {
                return Err(
                    "RX idle interval must be greater than zero and at most one hour".into(),
                )
            }
            _ => {}
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TriggerState {
    Armed,
    Capturing,
    Completed,
    Incomplete,
    Cancelled,
    Failed(String),
}
#[derive(Clone, Debug)]
pub struct TriggerStatus {
    pub state: TriggerState,
    pub buffered_bytes: usize,
    pub buffered_events: usize,
    pub events: u64,
    pub bytes: u64,
    pub dropped: u64,
    pub triggered: bool,
}
pub struct RollingCapture {
    options: CaptureOptions,
    matcher: Option<Matcher>,
    buffer: VecDeque<(Arc<TrafficEvent>, Instant)>,
    bytes: usize,
    match_tail: Vec<u8>,
    last_rx: Option<SystemTime>,
    started: Option<Instant>,
    post_bytes: usize,
    done: bool,
}
impl RollingCapture {
    pub fn new(options: CaptureOptions) -> Result<Self, String> {
        options.validate()?;
        let matcher = if let Trigger::Pattern(pattern) = &options.trigger {
            pattern.compile()?
        } else {
            None
        };
        Ok(Self {
            options,
            matcher,
            buffer: VecDeque::new(),
            bytes: 0,
            match_tail: Vec::new(),
            last_rx: None,
            started: None,
            post_bytes: 0,
            done: false,
        })
    }
    pub fn buffered(&self) -> (usize, usize) {
        (self.bytes, self.buffer.len())
    }
    pub fn triggered(&self) -> bool {
        self.started.is_some()
    }
    pub fn done(&self) -> bool {
        self.done
    }
    fn trim(&mut self, now: Instant) {
        while self.buffer.front().is_some_and(|(_, received)| {
            self.bytes > self.options.pre_bytes
                || self.buffer.len() > 2048
                || (!self.options.pre_duration.is_zero()
                    && now.saturating_duration_since(*received) > self.options.pre_duration)
        }) {
            self.bytes -= self.buffer.pop_front().unwrap().0.bytes.len();
        }
    }
    pub fn tick(&mut self, now: Instant) {
        self.trim(now);
        if self.started.is_some_and(|started| {
            !self.options.post_duration.is_zero()
                && now.saturating_duration_since(started) >= self.options.post_duration
        }) {
            self.done = true;
        }
    }
    pub fn manual(&mut self, now: Instant) -> Vec<Arc<TrafficEvent>> {
        if self.started.is_some() || self.done {
            return Vec::new();
        }
        self.begin(now)
    }
    fn begin(&mut self, now: Instant) -> Vec<Arc<TrafficEvent>> {
        self.trim(now);
        self.started = Some(now);
        self.done = self.options.post_bytes == 0 && self.options.post_duration.is_zero();
        self.bytes = 0;
        self.buffer.drain(..).map(|(event, _)| event).collect()
    }
    pub fn push(&mut self, event: Arc<TrafficEvent>, now: Instant) -> Vec<Arc<TrafficEvent>> {
        self.tick(now);
        if self.done {
            return Vec::new();
        }
        if self.started.is_some() {
            let remaining = if self.options.post_bytes == 0 {
                event.bytes.len()
            } else {
                self.options.post_bytes.saturating_sub(self.post_bytes)
            };
            let count = remaining.min(event.bytes.len());
            self.post_bytes += count;
            if self.options.post_bytes > 0 && self.post_bytes >= self.options.post_bytes {
                self.done = true;
            }
            if count == event.bytes.len() {
                return vec![event];
            }
            let clipped = Arc::new(TrafficEvent {
                bytes: Arc::from(&event.bytes[..count]),
                ..(*event).clone()
            });
            return (count > 0).then_some(clipped).into_iter().collect();
        }
        let mut triggered = false;
        if event.direction == Direction::Rx && !event.bytes.is_empty() {
            match &self.options.trigger {
                Trigger::Pattern(_) => {
                    let mut window = std::mem::take(&mut self.match_tail);
                    window.extend_from_slice(&event.bytes);
                    triggered = self.matcher.as_ref().unwrap().is_match(&window);
                    // Bounded cross-read context; regex cannot see unlimited stream history.
                    self.match_tail = window[window.len().saturating_sub(4095)..].to_vec();
                }
                Trigger::RxIdle(idle) => {
                    triggered = self
                        .last_rx
                        .and_then(|previous| event.timestamp.duration_since(previous).ok())
                        .is_some_and(|gap| gap >= *idle);
                }
                Trigger::Manual => {}
            }
            self.last_rx = Some(event.timestamp);
        }
        if triggered {
            let mut output = self.begin(now);
            output.push(event);
            return output;
        }
        self.bytes += event.bytes.len();
        self.buffer.push_back((event, now));
        self.trim(now);
        Vec::new()
    }
}
pub struct TriggeredCapture {
    pub path: PathBuf,
    stop: Arc<AtomicBool>,
    control: crate::traffic::TrafficControl,
    manual: Arc<AtomicBool>,
    status: Arc<Mutex<TriggerStatus>>,
    worker: Option<JoinHandle<()>>,
}
fn record(writer: &mut impl Write, value: &serde_json::Value) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, value).map_err(|e| e.to_string())?;
    writer.write_all(b"\n").map_err(|e| e.to_string())
}
fn write_events(
    writer: &mut BufWriter<std::fs::File>,
    events: Vec<Arc<TrafficEvent>>,
    progress: &Mutex<TriggerStatus>,
) -> Result<(), String> {
    for event in events {
        record(
            writer,
            &serde_json::json!({
                "type": "event", "sequence": event.sequence,
                "timestamp_unix_ns": timestamp_ns(event.timestamp).to_string(),
                "direction": if event.direction == Direction::Rx { "rx" } else { "tx" },
                "source_endpoint": event.endpoint.0, "raw_bytes": event.bytes.as_ref()
            }),
        )?;
        let mut status = progress.lock().unwrap();
        status.events += 1;
        status.bytes += event.bytes.len() as u64;
    }
    writer.flush().map_err(|error| error.to_string())
}
impl TriggeredCapture {
    pub fn start(
        path: &Path,
        endpoint: EndpointId,
        bus: &TrafficBus,
        options: CaptureOptions,
    ) -> Result<Self, String> {
        Self::start_subscription(
            path,
            endpoint.clone(),
            bus.subscribe_endpoint(64, endpoint),
            options,
        )
    }
    fn start_subscription(
        path: &Path,
        endpoint: EndpointId,
        subscription: crate::traffic::TrafficSubscription,
        options: CaptureOptions,
    ) -> Result<Self, String> {
        let mut rolling = RollingCapture::new(options)?;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        let control = subscription.control();
        let stop = Arc::new(AtomicBool::new(false));
        let manual = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(TriggerStatus {
            state: TriggerState::Armed,
            buffered_bytes: 0,
            buffered_events: 0,
            events: 0,
            bytes: 0,
            dropped: 0,
            triggered: false,
        }));
        let stopping = stop.clone();
        let firing = manual.clone();
        let progress = status.clone();
        let worker = thread::Builder::new().name("triggered-capture".into()).spawn(move || {
            let mut writer = BufWriter::new(file);
            let result = (|| -> Result<(), String> {
                record(&mut writer, &serde_json::json!({
                    "type": "header", "format": "signal-forge-terminal-capture", "version": 1,
                    "endpoint": endpoint.0, "started_unix_ns": timestamp_ns(SystemTime::now()).to_string(),
                    "semantics": "observed terminal RX/TX; bounded one-shot rolling capture"
                }))?;
                writer.flush().map_err(|error| error.to_string())?;
                while !stopping.load(Ordering::Acquire) && !rolling.done() {
                    let now = Instant::now();
                    if firing.swap(false, Ordering::AcqRel) {
                        write_events(&mut writer, rolling.manual(now), &progress)?;
                    }
                    rolling.tick(now);
                    if rolling.done() { break; }
                    match subscription.receiver.recv_timeout(Duration::from_millis(20)) {
                        Ok(event) => write_events(&mut writer, rolling.push(event, Instant::now()), &progress)?,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {},
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                    let mut status = progress.lock().unwrap();
                    (status.buffered_bytes, status.buffered_events) = rolling.buffered();
                    status.triggered = rolling.triggered();
                    status.dropped = subscription.dropped_events();
                    status.state = if rolling.triggered() { TriggerState::Capturing } else { TriggerState::Armed };
                }
                subscription.close();
                if rolling.triggered() && !rolling.done() {
                    for event in subscription.receiver.try_iter() {
                        write_events(&mut writer, rolling.push(event, Instant::now()), &progress)?;
                        if rolling.done() { break; }
                    }
                }
                let status = progress.lock().unwrap();
                record(&mut writer, &serde_json::json!({
                    "type": "footer", "events": status.events, "bytes": status.bytes,
                    "dropped_events": subscription.dropped_events(), "complete": subscription.dropped_events() == 0,
                    "triggered": rolling.triggered(), "ended_unix_ns": timestamp_ns(SystemTime::now()).to_string()
                }))?;
                writer.flush().map_err(|error| error.to_string())?;
                writer.get_ref().sync_all().map_err(|error| error.to_string())
            })();
            subscription.close();
            let mut status = progress.lock().unwrap();
            status.triggered = rolling.triggered();
            status.dropped = subscription.dropped_events();
            status.state = match result {
                Err(error) => TriggerState::Failed(error),
                Ok(()) if rolling.triggered() && status.dropped > 0 => TriggerState::Incomplete,
                Ok(()) if rolling.triggered() => TriggerState::Completed,
                Ok(()) => TriggerState::Cancelled,
            };
        }).map_err(|error| error.to_string())?;
        Ok(Self {
            path: path.to_path_buf(),
            stop,
            control,
            manual,
            status,
            worker: Some(worker),
        })
    }
    pub fn trigger(&self) {
        self.manual.store(true, Ordering::Release);
    }
    pub fn stop(&self) {
        self.control.close();
        self.stop.store(true, Ordering::Release);
    }
    pub fn status(&self) -> TriggerStatus {
        self.status.lock().unwrap().clone()
    }
    pub fn finish(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for TriggeredCapture {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overloaded_subscription_finishes_incomplete_with_exact_drop_count() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("capture.jsonl");
        let bus = TrafficBus::default();
        let endpoint = EndpointId("selected".into());
        let subscription = bus.subscribe_endpoint(1, endpoint.clone());
        bus.publish(endpoint.clone(), Direction::Rx, b"READY");
        bus.publish(endpoint.clone(), Direction::Tx, b"dropped");
        for _ in 0..100 {
            bus.publish(EndpointId("unrelated".into()), Direction::Rx, b"ignored");
        }
        let mut capture = TriggeredCapture::start_subscription(
            &path,
            endpoint,
            subscription,
            CaptureOptions {
                trigger: Trigger::Pattern(Pattern {
                    mode: crate::traffic_analysis::PatternMode::Text,
                    value: "READY".into(),
                }),
                post_bytes: 0,
                post_duration: Duration::ZERO,
                ..Default::default()
            },
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while capture.status().state != TriggerState::Incomplete {
            assert!(Instant::now() < deadline, "{:?}", capture.status());
            thread::sleep(Duration::from_millis(2));
        }
        capture.finish();
        let rows: Vec<serde_json::Value> = std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            rows[1]["raw_bytes"],
            serde_json::json!([82, 69, 65, 68, 89])
        );
        assert_eq!(rows.last().unwrap()["dropped_events"], 1);
        assert_eq!(rows.last().unwrap()["complete"], false);
        assert_eq!(capture.status().dropped, 1);
    }
}
