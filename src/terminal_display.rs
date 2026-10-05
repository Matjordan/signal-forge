//! Bounded terminal presentation. Traffic events and transport bytes stay untouched.
use crate::traffic::{Direction, TrafficEvent};
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, time::SystemTime};

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
#[derive(Debug, Clone)]
pub struct DisplayRow {
    pub timestamp: SystemTime,
    pub direction: Direction,
    pub bytes: Vec<u8>,
    pub complete: bool,
    pub truncated: bool,
}
pub struct LineDisplay {
    pub delimiter: LineDelimiter,
    pub rows: VecDeque<DisplayRow>,
    pub pending: Option<DisplayRow>,
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
        }
        self.rows.push_back(row);
    }
    fn pending_row(&mut self, timestamp: SystemTime) -> &mut DisplayRow {
        self.pending.get_or_insert_with(|| DisplayRow {
            timestamp,
            direction: Direction::Rx,
            bytes: Vec::new(),
            complete: false,
            truncated: false,
        })
    }
    fn finish(&mut self, timestamp: SystemTime) {
        self.pending_row(timestamp);
        let mut row = self.pending.take().unwrap();
        row.complete = true;
        self.push(row);
    }
    pub fn receive(&mut self, event: &TrafficEvent) {
        if event.direction == Direction::Tx {
            self.push(DisplayRow {
                timestamp: event.timestamp,
                direction: Direction::Tx,
                bytes: event.bytes.to_vec(),
                complete: true,
                truncated: false,
            });
            return;
        }
        for &byte in event.bytes.iter() {
            if self.delimiter == LineDelimiter::Auto && self.skip_lf {
                self.skip_lf = false;
                if byte == b'\n' {
                    continue;
                }
            }
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
                self.finish(event.timestamp);
                self.previous_cr = false;
                self.skip_lf = self.delimiter == LineDelimiter::Auto && byte == b'\r';
            } else {
                let row = self.pending_row(event.timestamp);
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
pub fn line_text(mut bytes: &[u8]) -> String {
    fn append(output: &mut String, text: &str) {
        for ch in text.chars() {
            match ch {
                '\r' => output.push_str("\\r"),
                '\n' => output.push_str("\\n"),
                '\t' => output.push_str("\\t"),
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
                append(&mut output, text);
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                append(&mut output, std::str::from_utf8(&bytes[..valid]).unwrap());
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
