//! Exact byte inspection: display glyphs map to byte ranges, never decorations.
use crate::{
    terminal_display::{self, SerialFraming, SourceRun},
    traffic::{self, Direction},
};
use std::{
    ops::Range,
    time::{Duration, SystemTime},
};

#[derive(Clone, Copy)]
pub enum RenderKind {
    Hex,
    Ascii,
    Unicode { controls: bool },
}
pub struct Glyph {
    pub columns: Range<usize>,
    pub bytes: Range<usize>,
}
pub struct MappedText {
    pub text: String,
    pub glyphs: Vec<Glyph>,
    columns: usize,
}
impl MappedText {
    fn append(&mut self, text: &str, bytes: Range<usize>) {
        let count = text.chars().count();
        self.glyphs.push(Glyph {
            columns: self.columns..self.columns + count,
            bytes,
        });
        self.text.push_str(text);
        self.columns += count;
    }
    /// Selecting any part of a glyph/escape/hex byte selects its entire byte range.
    /// Hex spacing and text outside the payload have no byte mapping.
    pub fn selected(&self, columns: Range<usize>) -> Option<Range<usize>> {
        if columns.is_empty() {
            return None;
        }
        let mut selected = self
            .glyphs
            .iter()
            .filter(|g| g.columns.start < columns.end && columns.start < g.columns.end);
        let first = selected.next()?;
        let end = selected.last().map_or(first.bytes.end, |g| g.bytes.end);
        Some(first.bytes.start..end)
    }
}
pub fn map_text(bytes: &[u8], kind: RenderKind) -> MappedText {
    let mut result = MappedText {
        text: String::new(),
        glyphs: Vec::new(),
        columns: 0,
    };
    match kind {
        RenderKind::Hex | RenderKind::Ascii => {
            for (i, &byte) in bytes.iter().enumerate() {
                if matches!(kind, RenderKind::Hex) && i != 0 {
                    result.text.push(' ');
                    result.columns += 1;
                }
                let text = if matches!(kind, RenderKind::Hex) {
                    format!("{byte:02X}")
                } else {
                    traffic::ascii(&[byte])
                };
                result.append(&text, i..i + 1);
            }
        }
        RenderKind::Unicode { controls } => {
            let mut offset = 0;
            while offset < bytes.len() {
                let remaining = &bytes[offset..];
                let (valid, invalid) = match std::str::from_utf8(remaining) {
                    Ok(_) => (remaining.len(), 0),
                    Err(e) => (
                        e.valid_up_to(),
                        e.error_len().unwrap_or(remaining.len() - e.valid_up_to()),
                    ),
                };
                for ch in std::str::from_utf8(&remaining[..valid]).unwrap().chars() {
                    let len = ch.len_utf8();
                    let data = &bytes[offset..offset + len];
                    let text = if controls {
                        terminal_display::control_text(data)
                    } else {
                        terminal_display::line_text(data)
                    };
                    // A visible Unicode glyph is atomic. Escaped multi-byte controls
                    // instead display individual bytes, each with its own mapping.
                    if ch.is_control() && text.starts_with("\\x") && len > 1 {
                        for (i, byte) in data.iter().enumerate() {
                            result.append(&format!("\\x{byte:02X}"), offset + i..offset + i + 1);
                        }
                    } else {
                        result.append(&text, offset..offset + len);
                    }
                    offset += len;
                }
                for _ in 0..invalid {
                    result.append(&format!("\\x{:02X}", bytes[offset]), offset..offset + 1);
                    offset += 1;
                }
            }
        }
    }
    result
}

#[derive(Clone, Debug)]
pub struct Piece {
    pub bytes: Range<usize>,
    pub direction: Direction,
    pub source: Option<SourceRun>,
}
#[derive(Default, Debug)]
pub struct Inspection {
    pub bytes: Vec<u8>,
    pub pieces: Vec<Piece>,
}
impl Inspection {
    /// Sources cover stored bytes only. Missing bounded provenance remains explicit.
    pub fn append(
        &mut self,
        bytes: &[u8],
        selected: Range<usize>,
        direction: Direction,
        sources: &[SourceRun],
    ) {
        let mut cursor = selected.start;
        for source in sources {
            let start = selected.start.max(source.start);
            let end = selected.end.min(source.start + source.len);
            if start >= end {
                continue;
            }
            if cursor < start {
                self.push(&bytes[cursor..start], direction, None);
            }
            let mut origin = source.clone();
            origin.stream_offset += (start - source.start) as u64;
            origin.len = end - start;
            origin.start = 0;
            self.push(&bytes[start..end], direction, Some(origin));
            cursor = end;
        }
        if cursor < selected.end {
            self.push(&bytes[cursor..selected.end], direction, None);
        }
    }
    fn push(&mut self, bytes: &[u8], direction: Direction, source: Option<SourceRun>) {
        let start = self.bytes.len();
        self.bytes.extend_from_slice(bytes);
        self.pieces.push(Piece {
            bytes: start..self.bytes.len(),
            direction,
            source,
        });
    }
    pub fn xor(&self) -> u8 {
        self.bytes.iter().fold(0, |sum, byte| sum ^ byte)
    }
    pub fn character_count(&self) -> Option<usize> {
        std::str::from_utf8(&self.bytes)
            .ok()
            .map(|s| s.chars().count())
    }
    pub fn ascii(&self) -> String {
        terminal_display::control_text(&self.bytes)
    }
    pub fn hex(&self) -> String {
        traffic::hex(&self.bytes)
    }
    pub fn missing_metadata(&self) -> bool {
        self.pieces.iter().any(|piece| piece.source.is_none())
    }
    pub fn boundaries(&self) -> Vec<usize> {
        let mut result = vec![];
        for pair in self.pieces.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            let contiguous = a.direction == b.direction
                && a.source
                    .as_ref()
                    .zip(b.source.as_ref())
                    .is_some_and(|(a, b)| {
                        a.epoch == b.epoch && a.stream_offset + a.len as u64 == b.stream_offset
                    });
            if !contiguous {
                result.push(b.bytes.start);
            }
        }
        result
    }
    pub fn segments(&self) -> Vec<Range<usize>> {
        if self.bytes.is_empty() {
            return vec![];
        }
        let mut starts = vec![0];
        starts.extend(self.boundaries());
        starts.push(self.bytes.len());
        starts.windows(2).map(|w| w[0]..w[1]).collect()
    }
    /// Timestamp order follows source event sequences, including interleaved line/TX rows.
    /// Never infer per-byte timestamps within a single read or TX event.
    pub fn event_times(&self) -> Option<(SystemTime, SystemTime)> {
        let first = self.pieces.first()?.source.as_ref()?;
        let (mut earliest, mut latest) = (first, first);
        for piece in &self.pieces {
            let source = piece.source.as_ref()?;
            if source.sequence < earliest.sequence {
                earliest = source;
            }
            if source.sequence > latest.sequence {
                latest = source;
            }
        }
        Some((earliest.timestamp, latest.timestamp))
    }
    pub fn observed_span(&self) -> Option<Duration> {
        let (first, last) = self.event_times()?;
        last.duration_since(first).ok()
    }
    pub fn wire_seconds(&self) -> Option<f64> {
        if self.bytes.is_empty() {
            return None;
        }
        self.pieces.iter().try_fold(0.0, |sum, p| {
            Some(
                sum + p
                    .source
                    .as_ref()?
                    .framing
                    .wire_seconds(p.bytes.len() as u64)?,
            )
        })
    }
    pub fn framing(&self) -> Vec<SerialFraming> {
        let mut values = vec![];
        for frame in self
            .pieces
            .iter()
            .filter_map(|p| p.source.as_ref().map(|s| s.framing))
        {
            if !values.contains(&frame) {
                values.push(frame);
            }
        }
        values
    }
    /// Validate each contiguous selected segment independently, never bridge a gap.
    pub fn nmea(&self) -> Vec<Nmea> {
        let unknown: Vec<_> = self
            .pieces
            .iter()
            .filter(|piece| piece.source.is_none())
            .map(|piece| &piece.bytes)
            .collect();
        self.segments()
            .into_iter()
            .filter(|range| {
                let index = unknown.partition_point(|piece| piece.end <= range.start);
                !unknown
                    .get(index)
                    .is_some_and(|piece| piece.start < range.end)
            })
            .flat_map(|range| {
                let start = range.start;
                nmea_sentences(&self.bytes[range])
                    .into_iter()
                    .map(move |mut sentence| {
                        sentence.bytes.start += start;
                        sentence.bytes.end += start;
                        sentence
                    })
            })
            .collect()
    }
}
#[derive(Debug, PartialEq, Eq)]
pub struct Nmea {
    pub bytes: Range<usize>,
    pub calculated: u8,
    pub received: u8,
}
impl Nmea {
    pub fn valid(&self) -> bool {
        self.calculated == self.received
    }
}
fn digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}
pub fn nmea_sentences(bytes: &[u8]) -> Vec<Nmea> {
    let mut result = vec![];
    for (start, byte) in bytes.iter().enumerate() {
        if !matches!(byte, b'$' | b'!') {
            continue;
        }
        let Some(relative) = bytes[start + 1..]
            .iter()
            .position(|b| *b == b'*' || matches!(b, b'$' | b'!' | b'\r' | b'\n'))
        else {
            continue;
        };
        let star = start + 1 + relative;
        if bytes[star] != b'*' || star == start + 1 {
            continue;
        }
        let Some(received) = bytes
            .get(star + 1)
            .and_then(|b| digit(*b))
            .zip(bytes.get(star + 2).and_then(|b| digit(*b)))
            .map(|(a, b)| a * 16 + b)
        else {
            continue;
        };
        if bytes
            .get(star + 3)
            .is_some_and(|b| !b.is_ascii_whitespace() && !matches!(b, b'$' | b'!'))
        {
            continue;
        }
        if !bytes[start + 1..star]
            .iter()
            .all(|b| (0x20..=0x7e).contains(b))
        {
            continue;
        }
        let calculated = bytes[start + 1..star].iter().fold(0, |sum, b| sum ^ b);
        result.push(Nmea {
            bytes: start..star + 3,
            calculated,
            received,
        });
    }
    result
}
