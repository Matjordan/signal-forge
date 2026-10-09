//! Matching and observed timing diagnostics. Never changes traffic or transport bytes.
use crate::{
    send::{self, Encoding, LineEnding},
    terminal_display::LineDelimiter,
    traffic::{Direction, TrafficEvent},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    time::{Duration, Instant, SystemTime},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PatternMode {
    #[default]
    Text,
    Hex,
    Regex,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pattern {
    pub mode: PatternMode,
    pub value: String,
}
pub enum Matcher {
    Bytes(Vec<u8>),
    Regex(regex::bytes::Regex),
}
impl Pattern {
    pub fn compile(&self) -> Result<Option<Matcher>, String> {
        if self.value.is_empty() {
            return Ok(None);
        }
        if self.value.len() > 4096 {
            return Err("Pattern exceeds 4096 bytes".into());
        }
        Ok(Some(match self.mode {
            PatternMode::Text => Matcher::Bytes(self.value.as_bytes().to_vec()),
            PatternMode::Hex => {
                let bytes = send::encode(&self.value, Encoding::Hex, false, LineEnding::None)
                    .map_err(|e| e.to_string())?;
                if bytes.is_empty() {
                    return Err("Enter at least one hex byte".into());
                }
                Matcher::Bytes(bytes)
            }
            PatternMode::Regex => Matcher::Regex(
                regex::bytes::RegexBuilder::new(&self.value)
                    .unicode(false)
                    .size_limit(1024 * 1024)
                    .dfa_size_limit(1024 * 1024)
                    .build()
                    .map_err(|e| e.to_string())?,
            ),
        }))
    }
}
impl Matcher {
    pub fn is_match(&self, bytes: &[u8]) -> bool {
        match self {
            Self::Bytes(pattern) => {
                !pattern.is_empty() && memchr::memmem::find(bytes, pattern).is_some()
            }
            Self::Regex(regex) => regex.is_match(bytes),
        }
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    #[default]
    Both,
    Rx,
    Tx,
}
impl Visibility {
    pub fn includes(self, direction: Direction) -> bool {
        matches!(
            (self, direction),
            (Self::Both, _) | (Self::Rx, Direction::Rx) | (Self::Tx, Direction::Tx)
        )
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Emphasis {
    #[default]
    Warning,
    Error,
    Ready,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Highlight {
    pub label: String,
    pub pattern: Pattern,
    pub emphasis: Emphasis,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewSettings {
    pub visibility: Visibility,
    pub direction_labels: bool,
    pub search: Pattern,
    pub filter: Pattern,
    pub highlights: Vec<Highlight>,
    pub delta_displayed: bool,
    pub delta_rx: bool,
    pub delta_tx: bool,
    pub line_span: bool,
    pub response_latency: bool,
}
impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            visibility: Visibility::Both,
            direction_labels: true,
            search: Pattern::default(),
            filter: Pattern::default(),
            highlights: Vec::new(),
            delta_displayed: false,
            delta_rx: false,
            delta_tx: false,
            line_span: false,
            response_latency: false,
        }
    }
}
#[derive(Debug, Clone, Default)]
pub struct RowTiming {
    pub displayed: Option<Duration>,
    pub rx: Option<Duration>,
    pub tx: Option<Duration>,
}
#[derive(Default)]
pub struct Timeline {
    displayed: Option<SystemTime>,
    rx: Option<SystemTime>,
    tx: Option<SystemTime>,
}
impl Timeline {
    /// Hidden rows update direction clocks, but never the previous displayed clock.
    pub fn observe(
        &mut self,
        timestamp: SystemTime,
        direction: Direction,
        visible: bool,
    ) -> RowTiming {
        let delta = |previous: Option<SystemTime>| {
            previous.and_then(|previous| timestamp.duration_since(previous).ok())
        };
        let timing = RowTiming {
            displayed: delta(self.displayed),
            rx: delta(self.rx),
            tx: delta(self.tx),
        };
        match direction {
            Direction::Rx => self.rx = Some(timestamp),
            Direction::Tx => self.tx = Some(timestamp),
        }
        if visible {
            self.displayed = Some(timestamp);
        }
        timing
    }
}
pub fn duration_text(duration: Duration) -> String {
    if duration.as_secs_f64() >= 1.0 {
        format!("{:.3} s", duration.as_secs_f64())
    } else if duration.as_micros() >= 1000 {
        format!("{:.2} ms", duration.as_secs_f64() * 1000.0)
    } else {
        format!("{} µs", duration.as_micros())
    }
}
#[derive(Clone, Debug)]
pub struct Latency {
    pub tx_sequence: u64,
    pub rx_sequence: u64,
    pub tx_timestamp: SystemTime,
    pub rx_timestamp: SystemTime,
    pub duration: Duration,
}
#[derive(Default, Clone, Copy)]
struct RateBucket {
    tick: Option<u64>,
    rx: u64,
    tx: u64,
}
pub struct Statistics {
    started: Instant,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_lines: u64,
    pub longest_rx_gap: Option<Duration>,
    pub last_latency: Option<Latency>,
    pub latency_count: u64,
    pub superseded_tx: u64,
    pub clock_regressions: u64,
    average_latency: f64,
    last_rx: Option<SystemTime>,
    pending_tx: Option<(u64, SystemTime)>,
    buckets: [RateBucket; 10],
    pub latencies: VecDeque<Latency>,
    skip_lf: bool,
    previous_cr: bool,
}
impl Default for Statistics {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}
impl Statistics {
    pub fn new(now: Instant) -> Self {
        Self {
            started: now,
            rx_bytes: 0,
            tx_bytes: 0,
            rx_lines: 0,
            longest_rx_gap: None,
            last_latency: None,
            latency_count: 0,
            superseded_tx: 0,
            clock_regressions: 0,
            average_latency: 0.0,
            last_rx: None,
            pending_tx: None,
            buckets: [RateBucket::default(); 10],
            latencies: VecDeque::new(),
            skip_lf: false,
            previous_cr: false,
        }
    }
    pub fn reset(&mut self, now: Instant) {
        let (skip_lf, previous_cr) = (self.skip_lf, self.previous_cr);
        *self = Self::new(now);
        self.skip_lf = skip_lf;
        self.previous_cr = previous_cr;
    }
    pub fn elapsed(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.started)
    }
    pub fn rates(&self, now: Instant) -> (u64, u64) {
        let tick = self.elapsed(now).as_millis() as u64 / 100;
        self.buckets
            .iter()
            .filter(|bucket| {
                bucket
                    .tick
                    .is_some_and(|value| value <= tick && tick - value < 10)
            })
            .fold((0, 0), |(rx, tx), bucket| (rx + bucket.rx, tx + bucket.tx))
    }
    pub fn average_latency(&self) -> Option<Duration> {
        (self.latency_count > 0).then(|| Duration::from_secs_f64(self.average_latency))
    }
    pub fn observe(&mut self, event: &TrafficEvent, now: Instant, delimiter: LineDelimiter) {
        let tick = self.elapsed(now).as_millis() as u64 / 100;
        let bucket = &mut self.buckets[(tick % 10) as usize];
        if bucket.tick != Some(tick) {
            *bucket = RateBucket {
                tick: Some(tick),
                ..Default::default()
            };
        }
        let count = event.bytes.len() as u64;
        match event.direction {
            Direction::Tx => {
                self.tx_bytes += count;
                bucket.tx += count;
                if count > 0 {
                    if self
                        .pending_tx
                        .replace((event.sequence, event.timestamp))
                        .is_some()
                    {
                        self.superseded_tx += 1;
                    }
                }
            }
            Direction::Rx => {
                self.rx_bytes += count;
                bucket.rx += count;
                if count == 0 {
                    return;
                }
                if let Some(previous) = self.last_rx {
                    if let Ok(gap) = event.timestamp.duration_since(previous) {
                        self.longest_rx_gap =
                            Some(self.longest_rx_gap.unwrap_or_default().max(gap));
                    } else {
                        self.clock_regressions += 1;
                    }
                }
                self.last_rx = Some(event.timestamp);
                if let Some((sequence, timestamp)) = self.pending_tx.take() {
                    if let Ok(duration) = event.timestamp.duration_since(timestamp) {
                        let latency = Latency {
                            tx_sequence: sequence,
                            rx_sequence: event.sequence,
                            tx_timestamp: timestamp,
                            rx_timestamp: event.timestamp,
                            duration,
                        };
                        self.latency_count += 1;
                        self.average_latency += (duration.as_secs_f64() - self.average_latency)
                            / self.latency_count as f64;
                        self.last_latency = Some(latency.clone());
                        if self.latencies.len() == 2000 {
                            self.latencies.pop_front();
                        }
                        self.latencies.push_back(latency);
                    } else {
                        self.clock_regressions += 1;
                    }
                }
                for &byte in event.bytes.iter() {
                    if delimiter == LineDelimiter::Auto && self.skip_lf {
                        self.skip_lf = false;
                        if byte == b'\n' {
                            continue;
                        }
                    }
                    let end = match delimiter {
                        LineDelimiter::Auto => byte == b'\r' || byte == b'\n',
                        LineDelimiter::Lf => byte == b'\n',
                        LineDelimiter::Cr => byte == b'\r',
                        LineDelimiter::CrLf => self.previous_cr && byte == b'\n',
                    };
                    if end {
                        self.rx_lines += 1;
                    }
                    self.skip_lf = delimiter == LineDelimiter::Auto && byte == b'\r';
                    self.previous_cr = byte == b'\r';
                }
            }
        }
    }
    pub fn delimiter_changed(&mut self) {
        self.skip_lf = false;
        self.previous_cr = false;
    }
}
