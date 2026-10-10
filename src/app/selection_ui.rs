//! Selection diagnostics are separate from terminal text copy and serial transport.
use super::*;
use signal_forge::{
    selection_inspector::{map_text, Inspection, RenderKind},
    terminal_display::SourceRun,
};

impl Terminal {
    fn inspection_key(&self) -> Option<Vec<u64>> {
        let (start, end) = self.selection.bounds()?;
        if start == end {
            return None;
        }
        let rows = if self.view.ready {
            self.view.rows.len()
        } else if self.receive_mode == ReceiveMode::Line {
            self.lines.len()
        } else {
            self.history.len()
        };
        let mut key = vec![
            start.row as u64,
            start.column as u64,
            end.row as u64,
            end.column as u64,
            self.receive_mode as u64,
            self.timestamps as u64,
            self.show_controls as u64,
            self.analysis.direction_labels as u64,
        ];
        for visible in start.row..=end.row.min(rows.saturating_sub(1)) {
            let source = self
                .view
                .rows
                .get(visible)
                .map_or(visible, |row| row.source);
            key.push(source as u64);
            if self.receive_mode == ReceiveMode::Line {
                let row = self.lines.row(source)?;
                key.extend([
                    row.id,
                    row.bytes.len() as u64,
                    row.terminator.len() as u64,
                    row.timing.as_ref().map_or(0, |t| t.byte_count),
                ]);
            } else {
                key.push(self.history.get(source)?.sequence);
            }
        }
        Some(key)
    }
    pub(super) fn inspect_selection(&self) -> Inspection {
        let mut result = Inspection::default();
        let Some((start, end)) = self.selection.bounds() else {
            return result;
        };
        let rows = if self.view.ready {
            self.view.rows.len()
        } else if self.receive_mode == ReceiveMode::Line {
            self.lines.len()
        } else {
            self.history.len()
        };
        for visible in start.row..=end.row.min(rows.saturating_sub(1)) {
            let source = self
                .view
                .rows
                .get(visible)
                .map_or(visible, |row| row.source);
            let (bytes, direction, sources) = if self.receive_mode == ReceiveMode::Line {
                let Some(row) = self.lines.row(source) else {
                    continue;
                };
                let mut bytes = row.bytes.clone();
                if self.show_controls {
                    bytes.extend_from_slice(&row.terminator);
                }
                (bytes, row.direction, row.sources.clone())
            } else {
                let Some(event) = self.history.get(source) else {
                    continue;
                };
                let sources = self
                    .history_origins
                    .get(source)
                    .zip(self.history_framing.get(source))
                    .map(|(&(stream_offset, epoch), &framing)| {
                        vec![SourceRun {
                            start: 0,
                            len: event.bytes.len(),
                            sequence: event.sequence,
                            timestamp: event.timestamp,
                            framing,
                            stream_offset,
                            epoch,
                        }]
                    })
                    .unwrap_or_default();
                (event.bytes.to_vec(), event.direction, sources)
            };
            let kind = if self.receive_mode == ReceiveMode::Hex {
                RenderKind::Hex
            } else if self.show_controls {
                RenderKind::Unicode { controls: true }
            } else if self.receive_mode == ReceiveMode::Line && direction == Direction::Rx {
                RenderKind::Unicode { controls: false }
            } else {
                RenderKind::Ascii
            };
            let mapped = map_text(&bytes, kind);
            let chunk = if self.receive_mode == ReceiveMode::Line {
                0
            } else {
                format!(" chunk #{}", self.history[source].sequence)
                    .chars()
                    .count()
            };
            let prefix = if self.timestamps { 13 } else { 0 }
                + if self.analysis.direction_labels { 2 } else { 0 }
                + chunk
                + if self.analysis.direction_labels || chunk != 0 {
                    2
                } else {
                    0
                };
            let Some(columns) = self.selection.columns(visible, usize::MAX) else {
                continue;
            };
            if let Some(range) = mapped
                .selected(columns.start.saturating_sub(prefix)..columns.end.saturating_sub(prefix))
            {
                result.append(&bytes, range, direction, &sources);
            }
        }
        result
    }
}
fn preview(ui: &mut egui::Ui, label: &str, mut text: String) {
    ui.label(label);
    ui.add(
        egui::TextEdit::multiline(&mut text)
            .font(egui::TextStyle::Monospace)
            .desired_width(f32::INFINITY)
            .desired_rows(3)
            .interactive(false),
    );
}
/// Cache full extraction and derived diagnostics, not just the preview strings.
/// A stable selection must not reformat or scan megabytes every GUI frame.
pub(super) struct CachedInspection {
    key: Vec<u64>,
    data: Inspection,
    segments: Vec<std::ops::Range<usize>>,
    nmea: Vec<signal_forge::selection_inspector::Nmea>,
    characters: Option<usize>,
    times: Option<(std::time::SystemTime, std::time::SystemTime)>,
    span: Option<Duration>,
    wire: Option<f64>,
    framing: Vec<SerialFraming>,
    missing: bool,
    xor: u8,
    ascii_preview: String,
    hex_preview: String,
}
impl CachedInspection {
    fn new(key: Vec<u64>, data: Inspection) -> Self {
        let times = data.event_times();
        let bytes = &data.bytes[..data.bytes.len().min(4096)];
        Self {
            key,
            segments: data.segments(),
            nmea: data.nmea(),
            characters: data.character_count(),
            times,
            span: times.and_then(|(first, last)| last.duration_since(first).ok()),
            wire: data.wire_seconds(),
            framing: data.framing(),
            missing: data.missing_metadata(),
            xor: data.xor(),
            ascii_preview: terminal_display::control_text(bytes),
            hex_preview: traffic::hex(bytes),
            data,
        }
    }
}
pub(super) fn show(ctx: &egui::Context, tab: &mut Terminal, traffic_area: egui::Rect) {
    let Some(key) = tab.inspection_key() else {
        tab.inspection_cache = None;
        tab.inspector_dismissed = None;
        return;
    };
    let bounds = tab.selection.bounds().unwrap();
    if tab.inspector_dismissed == Some(bounds) {
        return;
    }
    if tab
        .inspection_cache
        .as_ref()
        .is_none_or(|old| old.key != key)
    {
        let cache = CachedInspection::new(key, tab.inspect_selection());
        log::debug!(
            "Selection inspected: {} bytes, XOR {:02X}, {} segments, {} NMEA sentences",
            cache.data.bytes.len(),
            cache.xor,
            cache.segments.len(),
            cache.nmea.len()
        );
        tab.inspection_cache = Some(cache);
    }
    let cache = tab.inspection_cache.as_ref().unwrap();
    let inspection = &cache.data;
    let mut open = true;
    let window = egui::Window::new("Selection Inspector")
        .id(egui::Id::new("selection-inspector"))
        .open(&mut open)
        .constrain_to(traffic_area)
        .default_width(430.0)
        .default_height(660.0)
        .default_pos(egui::pos2(
            (ctx.input(|i| i.screen_rect().width()) - 460.0).max(0.0),
            80.0,
        ))
        .max_height(ctx.input(|i| i.screen_rect().height()) * 0.8)
        .vscroll(true)
        .interactable(!tab.selection.dragging);
    window.show(ctx, |ui| {
        ui.heading(format!("{} bytes", inspection.bytes.len()));
        match cache.characters {
            Some(chars) => { ui.label(format!("{chars} UTF-8 characters in selected bytes")); }
            None => { ui.weak("Character count unavailable: selected bytes are not valid UTF-8."); }
        }
        ui.weak("Escapes, Unicode glyphs and hex bytes select whole underlying bytes. Labels and timestamps are excluded.");
        if inspection.bytes.is_empty() {
            ui.label("Only display decorations are selected.");
            return;
        }
        let segments = &cache.segments;
        if segments.len() > 1 {
            ui.colored_label(theme::WARNING,format!("{} separate segments · previews and XOR concatenate only selected bytes",segments.len()));
        } else if cache.missing {
            ui.colored_label(theme::WARNING,"Stream continuity is not confirmed: source metadata is unavailable.");
        } else {
            ui.label("One contiguous range of retained stream bytes");
        }
        for (i,range) in segments.iter().enumerate().take(8) {
            let index = inspection.pieces.partition_point(|piece|piece.bytes.end <= range.start);
            let direction = inspection.pieces[index].direction;
            ui.small(format!("Segment {} · {:?} · bytes {}–{}",i+1,direction,range.start,range.end-1));
        }
        if segments.len() > 8 { ui.small(format!("{} more segments",segments.len()-8)); }
        ui.horizontal_wrapped(|ui| {
            if theme::button(ui,"Copy ASCII").clicked() { ui.ctx().copy_text(inspection.ascii()); }
            if theme::button(ui,"Copy Hex").clicked() { ui.ctx().copy_text(inspection.hex()); }
            if theme::button(ui,"Copy XOR").clicked() { ui.ctx().copy_text(format!("0x{:02X}",cache.xor)); }
        });
        preview(ui,"ASCII / Text · escaped non-printing bytes",cache.ascii_preview.clone());
        preview(ui,"Raw hex",cache.hex_preview.clone());
        if inspection.bytes.len() > 4096 {
            ui.weak("Preview limited to the first 4096 bytes. Copy actions include the complete selection.");
        }
        ui.separator();
        ui.label(RichText::new(format!("Selection XOR: 0x{:02X}",cache.xor)).strong());
        ui.weak("XOR includes every selected byte, including any selected delimiters or terminators.");
        let nmea = &cache.nmea;
        if nmea.is_empty() { ui.weak("No complete NMEA sentence in a verified contiguous segment. Selection XOR requires no framing."); }
        for sentence in nmea.iter().take(16) {
            ui.label(format!("NMEA sentence validation · bytes {}–{}",sentence.bytes.start,sentence.bytes.end-1));
            ui.monospace(format!("Calculated: 0x{:02X}    Received: 0x{:02X}",sentence.calculated,sentence.received));
            ui.colored_label(if sentence.valid() {theme::CONNECTED} else {theme::ERROR},if sentence.valid() {"VALID"} else {"INVALID"});
            if ui.push_id(sentence.bytes.start, |ui|theme::control(ui,&format!("Copy NMEA result {}",sentence.bytes.start),egui::Button::new("Copy NMEA result"))).inner.clicked() {
                ui.ctx().copy_text(format!("NMEA {} · calculated 0x{:02X} · received 0x{:02X}",
                    if sentence.valid() {"VALID"} else {"INVALID"},sentence.calculated,sentence.received));
            }
        }
        if nmea.len() > 16 { ui.small(format!("{} more NMEA sentences",nmea.len()-16)); }
        ui.separator();
        ui.label(RichText::new("Timing").strong());
        if let Some((first,last)) = cache.times {
            ui.small(format!("First source event: {}",signal_forge::inspector::timestamp_utc(first)));
            ui.small(format!("Last source event: {}",signal_forge::inspector::timestamp_utc(last)));
        }
        match cache.span {
            Some(span) => { ui.label(format!("Observed event span: {}",signal_forge::traffic_analysis::duration_text(span))); }
            None => { ui.weak("Observed span unavailable: missing event metadata or backwards clock."); }
        }
        match cache.wire {
            Some(seconds) => { ui.label(format!("Calculated serial wire time: {:.3} ms",seconds*1000.0)); }
            None => { ui.weak("Calculated wire time unavailable: missing or invalid framing."); }
        }
        for framing in &cache.framing { ui.small(framing.label()); }
        ui.weak("Observed timing comes from OS read / TX events, not individual bytes. Wire time is calculated from captured framing; it excludes gaps and is not measured latency.");
        if cache.missing {
            ui.colored_label(theme::WARNING,"Some selected bytes have no retained source metadata. Timing is unavailable for the complete selection.");
        }
    });
    if !open {
        tab.inspector_dismissed = Some(bounds);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use signal_forge::terminal_selection::Position;
    fn make_tab() -> Terminal {
        Terminal::restored(&signal_forge::workspace::SavedTerminal {
            settings: SerialSettings {
                path: "/dev/test".into(),
                ..Default::default()
            },
            show_controls: true,
            ..Default::default()
        })
    }
    fn receive(tab: &mut Terminal, sequence: u64, direction: Direction, bytes: &[u8]) {
        tab.receive(Arc::new(TrafficEvent {
            sequence,
            timestamp: UNIX_EPOCH + Duration::from_millis(sequence * 10),
            endpoint: tab.endpoint.id().clone(),
            direction,
            bytes: Arc::from(bytes),
        }));
    }
    fn select_all(tab: &mut Terminal) {
        let rows = if tab.view.ready {
            tab.view.rows.len()
        } else if tab.receive_mode == ReceiveMode::Line {
            tab.lines.len()
        } else {
            tab.history.len()
        };
        tab.selection.select_all(rows, usize::MAX);
    }
    #[test]
    fn same_exact_bytes_timing_and_xor_in_line_raw_and_hex_with_split_events() {
        let mut tab = make_tab();
        receive(&mut tab, 1, Direction::Rx, b"#0,");
        receive(&mut tab, 2, Direction::Rx, b"MED\0\xff\r\n");
        for mode in [ReceiveMode::Line, ReceiveMode::RawChunks, ReceiveMode::Hex] {
            tab.receive_mode = mode;
            tab.view.ready = false;
            select_all(&mut tab);
            let value = tab.inspect_selection();
            assert_eq!(value.bytes, b"#0,MED\0\xff\r\n");
            assert_eq!(value.segments().len(), 1);
            assert_eq!(value.observed_span(), Some(Duration::from_millis(10)));
            assert!((value.wire_seconds().unwrap() - 10.0 * 10.0 / 19200.0).abs() < 1e-12);
            assert_eq!(value.xor(), 0x8b);
        }
    }
    #[test]
    fn labels_timestamp_annotations_and_hidden_terminators_are_not_payload() {
        let mut tab = make_tab();
        tab.show_controls = false;
        receive(&mut tab, 1, Direction::Rx, b"$ABC*40\r\n");
        select_all(&mut tab);
        let value = tab.inspect_selection();
        assert_eq!(value.bytes, b"$ABC*40");
        assert!(value.nmea()[0].valid());
        tab.selection.anchor = Some(Position { row: 0, column: 0 });
        tab.selection.head = Some(Position { row: 0, column: 17 });
        assert!(tab.inspect_selection().bytes.is_empty());
        tab.selection.anchor = Some(Position { row: 0, column: 17 });
        tab.selection.head = Some(Position { row: 0, column: 19 });
        assert_eq!(tab.inspect_selection().bytes, b"$A");
        tab.timestamps = false;
        tab.analysis.direction_labels = false;
        tab.selection.anchor = Some(Position { row: 0, column: 1 });
        tab.selection.head = Some(Position { row: 0, column: 4 });
        assert_eq!(tab.inspect_selection().bytes, b"ABC");
    }
    #[test]
    fn filtered_rows_and_interleaved_rx_tx_have_explicit_boundaries_and_event_order() {
        let mut tab = make_tab();
        receive(&mut tab, 1, Direction::Rx, b"$AB");
        receive(&mut tab, 2, Direction::Tx, b"X");
        receive(&mut tab, 3, Direction::Rx, b"C*40\r\n");
        select_all(&mut tab);
        let value = tab.inspect_selection();
        assert_eq!(value.bytes, b"X$ABC*40\r\n");
        assert_eq!(value.segments().len(), 2);
        assert_eq!(
            value.event_times(),
            Some((
                UNIX_EPOCH + Duration::from_millis(10),
                UNIX_EPOCH + Duration::from_millis(30)
            ))
        );
        assert!(value.nmea()[0].valid());
        let mut tab = make_tab();
        receive(&mut tab, 1, Direction::Rx, b"KEEP\r\nDROP\r\nKEEP\r\n");
        tab.analysis.filter = signal_forge::traffic_analysis::Pattern {
            mode: signal_forge::traffic_analysis::PatternMode::Text,
            value: "KEEP".into(),
        };
        tab.refresh_view();
        select_all(&mut tab);
        let value = tab.inspect_selection();
        assert_eq!(value.bytes, b"KEEP\r\nKEEP\r\n");
        assert_eq!(value.segments().len(), 2);
    }
    #[test]
    fn clear_rebuild_and_history_eviction_keep_source_metadata_aligned() {
        let mut tab = make_tab();
        for sequence in 1..=2002 {
            receive(&mut tab, sequence, Direction::Rx, b"A\n");
        }
        assert_eq!(tab.history.len(), HISTORY_LIMIT);
        assert_eq!(tab.history_origins.len(), HISTORY_LIMIT);
        tab.receive_mode = ReceiveMode::Hex;
        tab.selection.anchor = Some(Position { row: 0, column: 0 });
        tab.selection.head = Some(Position {
            row: 0,
            column: usize::MAX,
        });
        assert_eq!(
            tab.inspect_selection().event_times().unwrap().0,
            UNIX_EPOCH + Duration::from_millis(30)
        );
        tab.rebuild_lines();
        assert_eq!(tab.history_origins.len(), tab.history.len());
    }
    #[test]
    fn raw_and_hex_metadata_columns_have_no_payload_bytes() {
        let mut tab = make_tab();
        receive(&mut tab, 1, Direction::Rx, b"$ABC*40");
        for mode in [ReceiveMode::RawChunks, ReceiveMode::Hex] {
            tab.receive_mode = mode;
            tab.selection.anchor = Some(Position { row: 0, column: 0 });
            tab.selection.head = Some(Position { row: 0, column: 26 });
            assert!(tab.inspect_selection().bytes.is_empty());
            tab.selection.anchor = Some(Position { row: 0, column: 26 });
            tab.selection.head = Some(Position { row: 0, column: 27 });
            assert_eq!(tab.inspect_selection().bytes, b"$");
        }
    }
    #[test]
    fn paused_monitoring_boundaries_survive_line_rebuilds() {
        let mut tab = make_tab();
        receive(&mut tab, 1, Direction::Rx, b"$AB");
        tab.paused = true;
        receive(&mut tab, 2, Direction::Tx, b"omitted");
        tab.paused = false;
        receive(&mut tab, 3, Direction::Rx, b"C*40\n");
        select_all(&mut tab);
        assert_eq!(tab.inspect_selection().segments().len(), 2);
        assert!(tab.inspect_selection().nmea().is_empty());
        tab.rebuild_lines();
        select_all(&mut tab);
        assert_eq!(tab.inspect_selection().segments().len(), 2);
        assert!(tab.inspect_selection().nmea().is_empty());
    }
}
