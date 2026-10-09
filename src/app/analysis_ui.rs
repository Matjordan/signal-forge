//! Cached presentation indexes and optional diagnostics for a terminal.
use super::*;
use signal_forge::traffic_analysis::{
    self as analysis, Emphasis, Matcher, Pattern, PatternMode, RowTiming, Timeline,
};

#[derive(Default)]
pub(super) struct ViewCache {
    pub ready: bool,
    pub dirty: bool,
    pub rows: Vec<ViewRow>,
    pub matches: Vec<usize>,
    pub current: Option<usize>,
    pub scroll_to: Option<usize>,
    pub errors: Vec<String>,
    compiled: Option<CompiledRules>,
}
pub(super) struct ViewRow {
    pub key: (bool, u64),
    pub source: usize,
    pub timing: RowTiming,
    pub matched: bool,
    pub emphasis: Option<(Emphasis, String)>,
    pub response: Option<(Duration, u64)>,
    pub span: Option<Duration>,
}
struct CompiledRules {
    settings: analysis::ViewSettings,
    search: Option<Matcher>,
    filter: Option<Matcher>,
    invalid_filter: bool,
    highlights: Vec<(analysis::Highlight, Matcher)>,
    errors: Vec<String>,
}
impl CompiledRules {
    fn new(settings: &analysis::ViewSettings) -> Self {
        let mut errors = Vec::new();
        let mut compile = |name: &str, pattern: &Pattern| -> Option<Matcher> {
            match pattern.compile() {
                Ok(matcher) => matcher,
                Err(error) => {
                    errors.push(format!("{name}: {error}"));
                    None
                }
            }
        };
        let search = compile("Search", &settings.search);
        let filter = compile("Filter", &settings.filter);
        let invalid_filter = !settings.filter.value.is_empty() && filter.is_none();
        let highlights = settings
            .highlights
            .iter()
            .take(32)
            .filter_map(|rule| {
                compile(&format!("Highlight {}", rule.label), &rule.pattern)
                    .map(|matcher| (rule.clone(), matcher))
            })
            .collect();
        if settings.highlights.len() > 32 {
            errors.push("Only the first 32 highlight rules are active".into());
        }
        Self {
            settings: settings.clone(),
            search,
            filter,
            invalid_filter,
            highlights,
            errors,
        }
    }
}
impl Terminal {
    pub(super) fn source_count(&self) -> usize {
        if self.receive_mode == ReceiveMode::Line {
            self.lines.len()
        } else {
            self.history.len()
        }
    }
    pub(super) fn source_row(&self, index: usize) -> (std::time::SystemTime, Direction, &[u8]) {
        if self.receive_mode == ReceiveMode::Line {
            let row = self.lines.row(index).unwrap();
            (row.timestamp, row.direction, &row.bytes)
        } else {
            let event = &self.history[index];
            (event.timestamp, event.direction, &event.bytes)
        }
    }
    pub(super) fn refresh_view(&mut self) {
        if self.view.ready
            && !self.view.dirty
            && self
                .view
                .compiled
                .as_ref()
                .is_some_and(|rules| rules.settings == self.analysis)
        {
            return;
        }
        if self
            .view
            .compiled
            .as_ref()
            .is_none_or(|rules| rules.settings != self.analysis)
        {
            self.view.compiled = Some(CompiledRules::new(&self.analysis));
        }
        let rules = self.view.compiled.as_ref().unwrap();
        let latencies: std::collections::BTreeMap<_, _> = self
            .statistics
            .latencies
            .iter()
            .map(|latency| (latency.rx_sequence, latency))
            .collect();
        let mut rows = Vec::new();
        let mut matches = Vec::new();
        let mut timeline = Timeline::default();
        let mut annotated = std::collections::HashSet::new();
        for index in 0..self.source_count() {
            let (timestamp, direction, payload) = self.source_row(index);
            let line =
                (self.receive_mode == ReceiveMode::Line).then(|| self.lines.row(index).unwrap());
            let combined;
            let bytes = if let Some(line) = line.filter(|line| !line.terminator.is_empty()) {
                combined = [payload, line.terminator.as_slice()].concat();
                combined.as_slice()
            } else {
                payload
            };
            let sequence_range = if let Some(line) = line {
                line.timing
                    .as_ref()
                    .map(|timing| (timing.first_sequence, timing.last_sequence))
            } else {
                Some((self.history[index].sequence, self.history[index].sequence))
            };
            let response = sequence_range
                .and_then(|(first, last)| {
                    latencies
                        .range(first..=last)
                        .find(|(sequence, _)| !annotated.contains(*sequence))
                })
                .map(|(&sequence, latency)| {
                    annotated.insert(sequence);
                    (latency.duration, latency.tx_sequence)
                });
            let visible = self.analysis.visibility.includes(direction)
                && !rules.invalid_filter
                && rules
                    .filter
                    .as_ref()
                    .is_none_or(|matcher| matcher.is_match(bytes));
            let timing = timeline.observe(timestamp, direction, visible);
            if !visible {
                continue;
            }
            let matched = rules
                .search
                .as_ref()
                .is_some_and(|matcher| matcher.is_match(bytes));
            if matched {
                matches.push(rows.len());
            }
            let emphasis = rules
                .highlights
                .iter()
                .find(|(_, matcher)| matcher.is_match(bytes))
                .map(|(rule, _)| (rule.emphasis, rule.label.clone()));
            let span = line
                .and_then(|line| line.timing.as_ref())
                .and_then(|timing| timing.observed_span());
            rows.push(ViewRow {
                key: if let Some(line) = line {
                    (true, line.id)
                } else {
                    (false, self.history[index].sequence)
                },
                source: index,
                timing,
                matched,
                emphasis,
                response,
                span,
            });
        }
        // Stable identities keep selection and search attached to the same traffic
        // when filtered rows shift, history rolls, or TX precedes a pending RX row.
        let positions: std::collections::HashMap<_, _> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| (row.key, index))
            .collect();
        if let Some((start, end)) = self.selection.bounds() {
            let retained: Vec<_> = self
                .view
                .rows
                .iter()
                .enumerate()
                .skip(start.row)
                .take(end.row.saturating_sub(start.row) + 1)
                .filter_map(|(old, row)| positions.get(&row.key).map(|&new| (old, new)))
                .collect();
            if let (Some(&(first_old, first_new)), Some(&(last_old, last_new))) =
                (retained.first(), retained.last())
            {
                let first = signal_forge::terminal_selection::Position {
                    row: first_new,
                    column: if first_old == start.row {
                        start.column
                    } else {
                        0
                    },
                };
                // An evicted tail is uncommon; clamp it to the surviving row's end.
                let last = signal_forge::terminal_selection::Position {
                    row: last_new,
                    column: if last_old == end.row {
                        end.column
                    } else {
                        usize::MAX
                    },
                };
                if self.selection.anchor <= self.selection.head {
                    self.selection.anchor = Some(first);
                    self.selection.head = Some(last);
                } else {
                    self.selection.anchor = Some(last);
                    self.selection.head = Some(first);
                }
            } else {
                self.selection.clear();
            }
        }
        self.view.current = self
            .view
            .current
            .and_then(|index| self.view.rows.get(index))
            .and_then(|old| positions.get(&old.key))
            .copied()
            .filter(|&index| rows[index].matched);
        if self.view.scroll_to.is_some() {
            self.view.scroll_to = self.view.current;
        }
        self.view.rows = rows;
        self.view.matches = matches;
        self.view.errors = rules.errors.clone();
        if self
            .view
            .current
            .is_some_and(|current| !self.view.matches.contains(&current))
        {
            self.view.current = None;
        }
        self.view.ready = true;
        self.view.dirty = false;
    }
    pub(super) fn navigate_match(&mut self, forward: bool) {
        self.refresh_view();
        let matches = &self.view.matches;
        if matches.is_empty() {
            return;
        }
        let position = self
            .view
            .current
            .and_then(|current| matches.iter().position(|&row| row == current));
        let next = match (position, forward) {
            (Some(position), true) => (position + 1) % matches.len(),
            (Some(position), false) => (position + matches.len() - 1) % matches.len(),
            (None, true) => 0,
            (None, false) => matches.len() - 1,
        };
        self.view.current = Some(matches[next]);
        self.view.scroll_to = self.view.current;
        self.auto_scroll = false;
    }
}
fn pattern_editor(ui: &mut egui::Ui, id: &str, pattern: &mut Pattern) {
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(id)
            .width(55.0)
            .selected_text(format!("{:?}", pattern.mode))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut pattern.mode, PatternMode::Text, "Text");
                ui.selectable_value(&mut pattern.mode, PatternMode::Hex, "Hex bytes");
                ui.selectable_value(&mut pattern.mode, PatternMode::Regex, "Byte regex");
            });
        ui.add(egui::TextEdit::singleline(&mut pattern.value).desired_width(180.0));
    });
}
impl TerminalViewer<'_> {
    pub(super) fn analysis_controls(ui: &mut egui::Ui, tab: &mut Terminal) {
        ui.push_id(("terminal-analysis", tab.endpoint.id().clone()), |ui| {
            let old = tab.analysis.clone();
            ui.menu_button("Analysis…", |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("analysis-options")
                    .max_height(
                        ui.ctx()
                            .input(|input| input.screen_rect().height() * 0.75)
                            .min(650.0),
                    )
                    .show(ui, |ui| {
                        ui.set_min_width(280.0);
                        ui.label("Search retained visible rows");
                        pattern_editor(ui, "search-pattern", &mut tab.analysis.search);
                        if tab.analysis != old {
                            tab.view.dirty = true;
                            tab.view.current = None;
                        }
                        tab.refresh_view();
                        ui.horizontal(|ui| {
                            if ui.button("Previous").clicked() {
                                tab.navigate_match(false);
                                ui.close_menu();
                            }
                            if ui.button("Next").clicked() {
                                tab.navigate_match(true);
                                ui.close_menu();
                            }
                            ui.label(format!("{} matching rows", tab.view.matches.len()));
                        });
                        ui.separator();
                        ui.label("Display only rows matching (empty = all)");
                        pattern_editor(ui, "filter-pattern", &mut tab.analysis.filter);
                        ui.separator();
                        ui.label("Highlight rules (first matching rule wins)");
                        let mut remove = None;
                        for (index, rule) in tab.analysis.highlights.iter_mut().enumerate() {
                            ui.push_id(index, |ui| {
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut rule.label)
                                            .desired_width(100.0)
                                            .hint_text("Label"),
                                    );
                                    egui::ComboBox::from_id_salt("emphasis")
                                        .selected_text(format!("{:?}", rule.emphasis))
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut rule.emphasis,
                                                Emphasis::Warning,
                                                "Warning",
                                            );
                                            ui.selectable_value(
                                                &mut rule.emphasis,
                                                Emphasis::Error,
                                                "Error",
                                            );
                                            ui.selectable_value(
                                                &mut rule.emphasis,
                                                Emphasis::Ready,
                                                "Ready",
                                            );
                                        });
                                    if ui.button("Remove").clicked() {
                                        remove = Some(index);
                                    }
                                });
                                pattern_editor(ui, "highlight-pattern", &mut rule.pattern);
                            });
                        }
                        if let Some(index) = remove {
                            tab.analysis.highlights.remove(index);
                        }
                        if ui
                            .add_enabled(
                                tab.analysis.highlights.len() < 32,
                                egui::Button::new("Add highlight rule"),
                            )
                            .clicked()
                        {
                            tab.analysis.highlights.push(Default::default());
                        }
                        ui.separator();
                        ui.label("Optional timing annotations");
                        ui.checkbox(
                            &mut tab.analysis.delta_displayed,
                            "Since previous displayed row",
                        );
                        ui.checkbox(&mut tab.analysis.delta_rx, "Since previous RX row");
                        ui.checkbox(&mut tab.analysis.delta_tx, "Since previous TX row");
                        ui.checkbox(&mut tab.analysis.line_span, "Assembled RX line duration");
                        ui.checkbox(&mut tab.analysis.response_latency, "TX → first RX latency");
                        ui.weak(
                            "Latest TX event wins; observed activity, not protocol correlation.",
                        );
                        for error in &tab.view.errors {
                            ui.colored_label(theme::ERROR, error);
                        }
                    });
            });
            if tab.analysis != old {
                tab.view.dirty = true;
                tab.selection.clear();
            }
            ui.menu_button("Statistics…", |ui| {
                let now = std::time::Instant::now();
                let stats = &tab.statistics;
                ui.label("Statistics since creation / last reset");
                let (rx, tx) = stats.rates(now);
                ui.label(format!("RX {} B · TX {} B", stats.rx_bytes, stats.tx_bytes));
                ui.label(format!("Recent RX {rx} B/s · TX {tx} B/s (1 s window)"));
                ui.label(format!(
                    "RX lines {} · elapsed {}",
                    stats.rx_lines,
                    analysis::duration_text(stats.elapsed(now))
                ));
                let value = |duration: Option<Duration>| {
                    duration
                        .map(analysis::duration_text)
                        .unwrap_or_else(|| "—".into())
                };
                ui.label(format!(
                    "Longest observed RX gap: {}",
                    value(stats.longest_rx_gap)
                ));
                ui.label(format!(
                    "Last TX → RX: {}",
                    value(stats.last_latency.as_ref().map(|latency| latency.duration))
                ));
                ui.label(format!(
                    "Mean TX → RX: {} ({} samples)",
                    value(stats.average_latency()),
                    stats.latency_count
                ));
                ui.label(format!(
                    "Superseded TX events: {} · clock regressions: {}",
                    stats.superseded_tx, stats.clock_regressions
                ));
                if let Some(latency) = &stats.last_latency {
                    ui.weak(format!(
                        "Latest pair: TX #{} → RX #{}",
                        latency.tx_sequence, latency.rx_sequence
                    ));
                }
                ui.weak("Observed monitor events; dropped events can make totals incomplete.");
                if ui.button("Reset statistics").clicked() {
                    tab.statistics.reset(now);
                    tab.view.dirty = true;
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tab() -> Terminal {
        Terminal::restored(&signal_forge::workspace::SavedTerminal {
            settings: SerialSettings {
                path: "/dev/test".into(),
                ..Default::default()
            },
            timestamps: false,
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
    #[test]
    fn visibility_filter_search_highlight_share_raw_payload_and_keep_hidden_history() {
        let mut tab = tab();
        receive(&mut tab, 1, Direction::Rx, b"REA");
        receive(&mut tab, 2, Direction::Tx, b"QUERY");
        receive(&mut tab, 3, Direction::Rx, b"DY\r\nERROR\0\xff\r\n");
        tab.analysis.visibility = analysis::Visibility::Rx;
        tab.analysis.direction_labels = false;
        tab.analysis.search = Pattern {
            mode: PatternMode::Hex,
            value: "00 FF".into(),
        };
        tab.analysis.highlights.push(analysis::Highlight {
            label: "Fault".into(),
            pattern: Pattern {
                mode: PatternMode::Regex,
                value: "ERROR.*".into(),
            },
            emphasis: Emphasis::Error,
        });
        tab.refresh_view();
        assert_eq!(tab.view.rows.len(), 2);
        assert_eq!(tab.view.matches, [1]);
        assert_eq!(
            tab.view.rows[1].emphasis.as_ref().unwrap().0,
            Emphasis::Error
        );
        assert_eq!(tab.history.len(), 3);
        assert_eq!(tab.lines.len(), 3);
        tab.analysis.filter = Pattern {
            mode: PatternMode::Text,
            value: "READY".into(),
        };
        tab.refresh_view();
        assert_eq!(tab.view.rows.len(), 1);
        assert_eq!(tab.source_row(tab.view.rows[0].source).2, b"READY");
        tab.analysis.filter.value.clear();
        tab.analysis.visibility = analysis::Visibility::Tx;
        tab.refresh_view();
        assert_eq!(tab.view.rows.len(), 1);
        assert_eq!(tab.source_row(tab.view.rows[0].source).2, b"QUERY");
        tab.analysis.visibility = analysis::Visibility::Both;
        tab.refresh_view();
        assert_eq!(tab.view.rows.len(), 3);
        assert_eq!(tab.history[2].bytes.as_ref(), b"DY\r\nERROR\0\xff\r\n");
        tab.analysis.filter = Pattern {
            mode: PatternMode::Regex,
            value: "[".into(),
        };
        tab.refresh_view();
        assert!(tab.view.rows.is_empty());
        assert!(!tab.view.errors.is_empty());
    }
    #[test]
    fn navigation_wraps_and_binary_regex_searches_hex_and_raw_views() {
        let mut tab = tab();
        for seq in 1..=10 {
            receive(
                &mut tab,
                seq,
                Direction::Rx,
                if seq % 2 == 0 { b"\0\xff\n" } else { b"OK\n" },
            );
        }
        tab.analysis.search = Pattern {
            mode: PatternMode::Regex,
            value: r"\x00\xFF".into(),
        };
        for mode in [ReceiveMode::Line, ReceiveMode::RawChunks, ReceiveMode::Hex] {
            tab.receive_mode = mode;
            tab.view.dirty = true;
            tab.view.current = None;
            tab.refresh_view();
            assert_eq!(tab.view.matches, [1, 3, 5, 7, 9]);
            tab.navigate_match(false);
            assert_eq!(tab.view.current, Some(9));
            tab.navigate_match(true);
            assert_eq!(tab.view.current, Some(1));
            tab.navigate_match(true);
            assert_eq!(tab.view.current, Some(3));
            assert_eq!(tab.view.scroll_to, Some(3));
            assert!(!tab.auto_scroll);
        }
    }
    #[test]
    fn paused_display_still_counts_traffic_and_recording_ignores_all_view_settings() {
        let mut tab = tab();
        tab.paused = true;
        tab.analysis.visibility = analysis::Visibility::Tx;
        tab.analysis.filter = Pattern {
            mode: PatternMode::Text,
            value: "never match".into(),
        };
        let bus = TrafficBus::default();
        let events = bus.subscribe(10);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rx.bin");
        let mut recording = signal_forge::raw_recording::RawRecording::start(
            &path,
            tab.endpoint.id().clone(),
            &bus,
        )
        .unwrap();
        for (direction, bytes) in [
            (Direction::Tx, b"REQUEST".as_slice()),
            (Direction::Rx, b"\0\xff\r\n".as_slice()),
        ] {
            bus.publish(tab.endpoint.id().clone(), direction, bytes);
            tab.receive(events.recv_timeout(Duration::from_secs(1)).unwrap());
        }
        recording.finish();
        assert_eq!(std::fs::read(path).unwrap(), b"\0\xff\r\n");
        assert_eq!(
            (
                tab.statistics.rx_bytes,
                tab.statistics.tx_bytes,
                tab.statistics.rx_lines
            ),
            (4, 7, 1)
        );
        assert_eq!(tab.statistics.latency_count, 1);
        assert!(tab.history.is_empty());
        tab.paused = false;
        receive(&mut tab, 10, Direction::Rx, b"next\n");
        let history = tab.history.len();
        tab.statistics.reset(std::time::Instant::now());
        assert_eq!(tab.history.len(), history);
        assert_eq!(tab.statistics.rx_bytes, 0);
    }
    #[test]
    fn filtered_live_selection_and_search_follow_same_rows_through_eviction() {
        use signal_forge::terminal_selection::Position;
        let mut tab = tab();
        tab.analysis.visibility = analysis::Visibility::Rx;
        tab.analysis.search = Pattern {
            mode: PatternMode::Text,
            value: "READY".into(),
        };
        for seq in 1..=2000 {
            receive(
                &mut tab,
                seq,
                Direction::Rx,
                if seq == 2 { b"READY\n" } else { b"OK\n" },
            );
        }
        tab.refresh_view();
        tab.navigate_match(true);
        assert_eq!(tab.view.current, Some(1));
        tab.selection.anchor = Some(Position { row: 1, column: 0 });
        tab.selection.head = Some(Position { row: 1, column: 5 });
        tab.selection.dragging = true;
        receive(&mut tab, 2001, Direction::Tx, b"hidden TX");
        tab.refresh_view();
        assert!(tab.selection.dragging);
        assert_eq!(tab.selection.anchor.unwrap().row, 0);
        assert_eq!(tab.view.current, Some(0));
        assert_eq!(tab.view.scroll_to, Some(0));
        assert_eq!(tab.source_row(tab.view.rows[0].source).2, b"READY");
        receive(&mut tab, 2002, Direction::Rx, b"NEW\n");
        tab.refresh_view();
        assert!(tab.selection.bounds().is_none());
        assert!(tab.view.current.is_none());
    }
    #[test]
    fn pending_rx_selection_keeps_its_text_when_tx_inserts_a_row() {
        use signal_forge::terminal_selection::Position;
        let mut tab = tab();
        receive(&mut tab, 1, Direction::Rx, b"PENDING");
        tab.refresh_view();
        tab.selection.anchor = Some(Position { row: 0, column: 1 });
        tab.selection.head = Some(Position { row: 0, column: 4 });
        receive(&mut tab, 2, Direction::Tx, b"REQUEST");
        tab.refresh_view();
        assert_eq!(tab.selection.anchor.unwrap().row, 1);
        assert_eq!(tab.selection.head.unwrap().row, 1);
        assert_eq!(tab.source_row(tab.view.rows[1].source).2, b"PENDING");
    }

    #[test]
    fn response_annotation_attaches_once_to_partial_line_that_spans_tx_and_rx() {
        let mut tab = tab();
        receive(&mut tab, 1, Direction::Rx, b"PART");
        receive(&mut tab, 2, Direction::Tx, b"REQUEST");
        receive(&mut tab, 3, Direction::Rx, b"IAL\nANOTHER\n");
        tab.refresh_view();
        let responses: Vec<_> = tab
            .view
            .rows
            .iter()
            .filter_map(|row| row.response)
            .collect();
        assert_eq!(responses, [(Duration::from_millis(10), 2)]);
        assert_eq!(
            tab.view.rows[1].response,
            Some((Duration::from_millis(10), 2))
        );
    }
}
