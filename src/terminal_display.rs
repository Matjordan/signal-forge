//! Bounded terminal presentation. Traffic events and transport bytes stay untouched.
use crate::{
    config::{Parity, SerialSettings},
    traffic::{Direction, TrafficEvent},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    time::{Duration, SystemTime},
};

pub const ROW_LIMIT: usize = 2000;
pub const LINE_BYTE_LIMIT: usize = 64 * 1024;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReceiveMode {
    #[default]
    Line,
    RawChunks,
    Hex,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineDelimiter {
    #[default]
    Auto,
    Lf,
    CrLf,
    Cr,
}
/// Serial framing captured at receipt, independent of later settings edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SerialFraming {
    pub baud: u32,
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: u8,
}
impl From<&SerialSettings> for SerialFraming {
    fn from(settings: &SerialSettings) -> Self {
        Self {
            baud: settings.baud,
            data_bits: settings.data_bits,
            parity: settings.parity,
            stop_bits: settings.stop_bits,
        }
    }
}
impl SerialFraming {
    pub fn wire_seconds(self, byte_count: u64) -> Option<f64> {
        (self.baud > 0 && (5..=8).contains(&self.data_bits) && (1..=2).contains(&self.stop_bits))
            .then(|| {
                byte_count as f64
                    * (1 + self.data_bits + u8::from(self.parity != Parity::None) + self.stop_bits)
                        as f64
                    / self.baud as f64
            })
    }
    pub fn label(self) -> String {
        let parity = match self.parity {
            Parity::None => 'N',
            Parity::Odd => 'O',
            Parity::Even => 'E',
        };
        format!(
            "{} baud · {}{}{}",
            self.baud, self.data_bits, parity, self.stop_bits
        )
    }
}
#[derive(Debug, Clone)]
pub struct RxTiming {
    pub first_timestamp: SystemTime,
    pub last_timestamp: SystemTime,
    /// All contributing bytes, including delimiters and display-truncated bytes.
    pub byte_count: u64,
    pub read_count: u64,
    pub framing: SerialFraming,
    pub mixed_framing: bool,
    wire_seconds: Option<f64>,
    last_sequence: u64,
}
impl RxTiming {
    fn new(event: &TrafficEvent, framing: SerialFraming) -> Self {
        Self {
            first_timestamp: event.timestamp,
            last_timestamp: event.timestamp,
            byte_count: 0,
            read_count: 1,
            framing,
            mixed_framing: false,
            wire_seconds: Some(0.0),
            last_sequence: event.sequence,
        }
    }
    fn observe_byte(&mut self, event: &TrafficEvent, framing: SerialFraming) {
        self.byte_count += 1;
        self.last_timestamp = event.timestamp;
        if event.sequence != self.last_sequence {
            self.read_count += 1;
            self.last_sequence = event.sequence;
        }
        self.mixed_framing |= framing != self.framing;
        self.wire_seconds = self
            .wire_seconds
            .zip(framing.wire_seconds(1))
            .map(|(sum, next)| sum + next);
    }
    pub fn observed_span(&self) -> Option<Duration> {
        self.last_timestamp
            .duration_since(self.first_timestamp)
            .ok()
    }
    pub fn calculated_wire_seconds(&self) -> Option<f64> {
        self.wire_seconds
    }
    pub fn tooltip(&self) -> String {
        let observed = match self.observed_span() {
            Some(span) => format!(
                "{:.1} ms{}",
                span.as_secs_f64() * 1000.0,
                if self.read_count == 1 {
                    " / single read"
                } else {
                    ""
                }
            ),
            None => "unavailable / clock moved backwards".into(),
        };
        let wire = self
            .wire_seconds
            .map(|seconds| format!("{:.1} ms", seconds * 1000.0))
            .unwrap_or_else(|| "unavailable / invalid framing".into());
        let framing = if self.mixed_framing {
            format!("Mixed settings · starting at {}", self.framing.label())
        } else {
            self.framing.label()
        };
        format!("{} bytes (including line endings)\nObserved RX span: {observed}\nCalculated wire time: {wire}\n{framing}\nObserved span is between OS read events; wire time is theoretical.", self.byte_count)
    }
}
#[derive(Debug, Clone)]
pub struct DisplayRow {
    pub timestamp: SystemTime,
    pub direction: Direction,
    pub bytes: Vec<u8>,
    pub complete: bool,
    /// Actual recognized delimiter, separate from the logical payload.
    pub terminator: Vec<u8>,
    pub truncated: bool,
    pub timing: Option<RxTiming>,
}
pub struct LineDisplay {
    pub delimiter: LineDelimiter,
    pub rows: VecDeque<DisplayRow>,
    pub pending: Option<DisplayRow>,
    pub evicted_rows: usize,
    skip_lf: bool,
    previous_cr: bool,
}
impl Default for LineDisplay {
    fn default() -> Self {
        Self::new(LineDelimiter::Auto)
    }
}
impl LineDisplay {
    pub fn new(delimiter: LineDelimiter) -> Self {
        Self {
            delimiter,
            rows: VecDeque::new(),
            pending: None,
            evicted_rows: 0,
            skip_lf: false,
            previous_cr: false,
        }
    }
    pub fn clear(&mut self) {
        *self = Self::new(self.delimiter);
    }
    /// A display gap must not join RX bytes from opposite sides of a pause.
    pub fn discard_pending(&mut self) {
        self.pending = None;
        self.skip_lf = false;
        self.previous_cr = false;
    }
    fn push(&mut self, row: DisplayRow) {
        if self.rows.len() == ROW_LIMIT {
            self.rows.pop_front();
            self.evicted_rows += 1;
        }
        self.rows.push_back(row);
    }
    fn pending_row(&mut self, event: &TrafficEvent, framing: SerialFraming) -> &mut DisplayRow {
        self.pending.get_or_insert_with(|| DisplayRow {
            timestamp: event.timestamp,
            direction: Direction::Rx,
            bytes: Vec::new(),
            complete: false,
            terminator: Vec::new(),
            truncated: false,
            timing: Some(RxTiming::new(event, framing)),
        })
    }
    fn finish(&mut self) {
        let mut row = self.pending.take().unwrap();
        row.complete = true;
        self.push(row);
    }
    /// Convenience for the default 19200/8N1 framing. Live terminals must use
    /// `receive_with_framing` with the settings applied to their open endpoint.
    pub fn receive(&mut self, event: &TrafficEvent) {
        self.receive_with_framing(event, SerialFraming::from(&SerialSettings::default()));
    }
    pub fn receive_with_framing(&mut self, event: &TrafficEvent, framing: SerialFraming) {
        if event.direction == Direction::Tx {
            self.push(DisplayRow {
                timestamp: event.timestamp,
                direction: Direction::Tx,
                bytes: event.bytes.to_vec(),
                complete: true,
                terminator: Vec::new(),
                truncated: false,
                timing: None,
            });
            return;
        }
        for &byte in event.bytes.iter() {
            if self.delimiter == LineDelimiter::Auto && self.skip_lf {
                self.skip_lf = false;
                if byte == b'\n' {
                    // Auto finishes on CR immediately; a later LF belongs to that
                    // same line even when TX rows intervene or its payload is empty.
                    if let Some(row) = self
                        .rows
                        .iter_mut()
                        .rev()
                        .find(|row| row.direction == Direction::Rx)
                    {
                        row.timing.as_mut().unwrap().observe_byte(event, framing);
                        row.terminator.push(b'\n');
                    }
                    continue;
                }
            }
            self.pending_row(event, framing)
                .timing
                .as_mut()
                .unwrap()
                .observe_byte(event, framing);
            let end = match self.delimiter {
                LineDelimiter::Auto => byte == b'\r' || byte == b'\n',
                LineDelimiter::Lf => byte == b'\n',
                LineDelimiter::Cr => byte == b'\r',
                LineDelimiter::CrLf => self.previous_cr && byte == b'\n',
            };
            if end {
                if self.delimiter == LineDelimiter::CrLf {
                    if let Some(row) = &mut self.pending {
                        if !row.truncated {
                            row.bytes.pop();
                        }
                    }
                }
                self.pending.as_mut().unwrap().terminator = if self.delimiter == LineDelimiter::CrLf
                {
                    b"\r\n".to_vec()
                } else {
                    vec![byte]
                };
                self.finish();
                self.previous_cr = false;
                self.skip_lf = self.delimiter == LineDelimiter::Auto && byte == b'\r';
            } else {
                let row = self.pending_row(event, framing);
                if row.bytes.len() < LINE_BYTE_LIMIT {
                    row.bytes.push(byte);
                } else {
                    row.truncated = true;
                }
                self.previous_cr = byte == b'\r';
            }
        }
    }
    pub fn len(&self) -> usize {
        self.rows.len() + usize::from(self.pending.is_some())
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn row(&self, index: usize) -> Option<&DisplayRow> {
        self.rows.get(index).or_else(|| {
            (index == self.rows.len())
                .then_some(self.pending.as_ref())
                .flatten()
        })
    }
}
/// Show valid Unicode, escape controls and invalid bytes without replacement characters.
pub fn line_text(bytes: &[u8]) -> String {
    render_text(bytes, false)
}
/// Explicit visible control notation, preserving valid Unicode and invalid bytes.
pub fn control_text(bytes: &[u8]) -> String {
    render_text(bytes, true)
}
fn render_text(mut bytes: &[u8], controls: bool) -> String {
    fn append(output: &mut String, text: &str, controls: bool) {
        for ch in text.chars() {
            match ch {
                '\r' => output.push_str("\\r"),
                '\n' => output.push_str("\\n"),
                '\t' => output.push_str("\\t"),
                '\0' if controls => output.push_str("\\0"),
                ch if ch.is_control() => {
                    for byte in ch.to_string().bytes() {
                        output.push_str(&format!("\\x{byte:02X}"));
                    }
                }
                ch => output.push(ch),
            }
        }
    }
    let mut output = String::new();
    while !bytes.is_empty() {
        match std::str::from_utf8(bytes) {
            Ok(text) => {
                append(&mut output, text, controls);
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                append(
                    &mut output,
                    std::str::from_utf8(&bytes[..valid]).unwrap(),
                    controls,
                );
                let count = error.error_len().unwrap_or(bytes.len() - valid);
                for byte in &bytes[valid..valid + count] {
                    output.push_str(&format!("\\x{byte:02X}"));
                }
                bytes = &bytes[valid + count..];
            }
        }
    }
    output
}
