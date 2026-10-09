//! Terminal presentation sections; transport actions remain on the existing endpoints.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TerminalTool {
    Send,
    Repeat,
    Presets,
    Files,
    Capture,
}

impl TerminalViewer<'_> {
    pub(super) fn selection(&mut self, ui: &mut egui::Ui, tab: &mut Terminal) {
        if ui.rect_contains_pointer(ui.max_rect()) && ui.input(|input| input.pointer.any_pressed())
        {
            *self.selected = Some(tab.endpoint.id().clone());
        }
    }

    pub(super) fn serial_settings(&mut self, ui: &mut egui::Ui, tab: &mut Terminal) {
        ui.push_id(tab.endpoint.id().0.clone(), |ui| {
            ui.horizontal_wrapped(|ui| {
                let connected = matches!(
                    tab.endpoint.state(),
                    ConnectionState::Connected | ConnectionState::Connecting
                );
                ui.add_enabled_ui(!connected, |ui| {
                    tab.baud_control.ui(ui, &mut tab.settings.baud);
                    egui::ComboBox::from_id_salt("pane_bits")
                        .width(40.0)
                        .selected_text(format!("{} bits", tab.settings.data_bits))
                        .show_ui(ui, |ui| {
                            for value in 5..=8 {
                                ui.selectable_value(
                                    &mut tab.settings.data_bits,
                                    value,
                                    value.to_string(),
                                );
                            }
                        });
                    egui::ComboBox::from_id_salt("pane_parity")
                        .width(55.0)
                        .selected_text(format!("{:?}", tab.settings.parity))
                        .show_ui(ui, |ui| {
                            for value in [Parity::None, Parity::Odd, Parity::Even] {
                                ui.selectable_value(
                                    &mut tab.settings.parity,
                                    value,
                                    format!("{value:?}"),
                                );
                            }
                        });
                    egui::ComboBox::from_id_salt("pane_stop")
                        .width(45.0)
                        .selected_text(format!("{} stop", tab.settings.stop_bits))
                        .show_ui(ui, |ui| {
                            for value in 1..=2 {
                                ui.selectable_value(
                                    &mut tab.settings.stop_bits,
                                    value,
                                    value.to_string(),
                                );
                            }
                        });
                    egui::ComboBox::from_id_salt("pane_flow")
                        .width(60.0)
                        .selected_text(format!("{:?}", tab.settings.flow))
                        .show_ui(ui, |ui| {
                            for value in [
                                FlowControl::None,
                                FlowControl::Hardware,
                                FlowControl::Software,
                            ] {
                                ui.selectable_value(
                                    &mut tab.settings.flow,
                                    value,
                                    format!("{value:?}"),
                                );
                            }
                        });
                });
            });
        });
    }

    pub(super) fn display_controls(&mut self, ui: &mut egui::Ui, tab: &mut Terminal) {
        let previous_mode = tab.receive_mode;
        let old_analysis = tab.analysis.clone();
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut tab.receive_mode, ReceiveMode::Line, "Line");
            ui.selectable_value(&mut tab.receive_mode, ReceiveMode::RawChunks, "Raw Chunks");
            ui.selectable_value(&mut tab.receive_mode, ReceiveMode::Hex, "Hex");
            ui.menu_button("Display…", |ui| {
                let previous = tab.lines.delimiter;
                egui::ComboBox::from_id_salt((tab.endpoint.id().0.clone(), "receive-delimiter"))
                    .width(55.0)
                    .selected_text(match tab.lines.delimiter {
                        LineDelimiter::Auto => "Auto",
                        LineDelimiter::Lf => "LF",
                        LineDelimiter::CrLf => "CRLF",
                        LineDelimiter::Cr => "CR",
                    })
                    .show_ui(ui, |ui| {
                        for (value, label) in [
                            (LineDelimiter::Auto, "Auto"),
                            (LineDelimiter::Lf, "LF"),
                            (LineDelimiter::CrLf, "CRLF"),
                            (LineDelimiter::Cr, "CR"),
                        ] {
                            ui.selectable_value(&mut tab.lines.delimiter, value, label);
                        }
                    });
                if previous != tab.lines.delimiter {
                    tab.rebuild_lines();
                    tab.statistics.delimiter_changed();
                }
                let old = (tab.timestamps, tab.show_controls);
                egui::ComboBox::from_id_salt(("traffic-visibility", tab.endpoint.id().clone()))
                    .selected_text(match tab.analysis.visibility {
                        signal_forge::traffic_analysis::Visibility::Both => "RX + TX",
                        signal_forge::traffic_analysis::Visibility::Rx => "RX only",
                        signal_forge::traffic_analysis::Visibility::Tx => "TX only",
                    })
                    .show_ui(ui, |ui| {
                        use signal_forge::traffic_analysis::Visibility;
                        ui.selectable_value(
                            &mut tab.analysis.visibility,
                            Visibility::Both,
                            "RX + TX",
                        );
                        ui.selectable_value(
                            &mut tab.analysis.visibility,
                            Visibility::Rx,
                            "RX only",
                        );
                        ui.selectable_value(
                            &mut tab.analysis.visibility,
                            Visibility::Tx,
                            "TX only",
                        );
                    });
                ui.checkbox(&mut tab.analysis.direction_labels, "Show RX/TX labels");
                ui.checkbox(&mut tab.timestamps, "Timestamps");
                ui.checkbox(&mut tab.show_controls, "Show Control Characters");
                if old != (tab.timestamps, tab.show_controls) {
                    tab.selection.clear();
                }
                if ui.button("Copy All").clicked() {
                    tab.view.dirty = true;
                    tab.refresh_view();
                    let rows = Self::row_count(tab);
                    let text = (0..rows)
                        .map(|index| Self::traffic_layout(tab, index).text)
                        .collect::<Vec<_>>()
                        .join("\n");
                    ui.ctx().copy_text(text);
                    ui.close_menu();
                }
                ui.checkbox(&mut tab.auto_scroll, "Auto-scroll");
                ui.checkbox(&mut tab.paused, "Pause display");
                if ui.button("Clear").clicked() {
                    tab.selection.clear();
                    tab.view.dirty = true;
                    tab.history.clear();
                    tab.history_framing.clear();
                    tab.lines.clear();
                    tab.rx_breaks.clear();
                    tab.rx_gap = false;
                }
            });
            Self::analysis_controls(ui, tab);
            if tab.paused {
                ui.colored_label(theme::WARNING, "Paused");
            }
            if ui.available_width() >= 110.0 {
                ui.label(
                    RichText::new(format!("RX {} B · TX {} B", tab.rx_bytes, tab.tx_bytes))
                        .small()
                        .color(theme::MUTED),
                );
            }
        });
        if previous_mode != tab.receive_mode || old_analysis != tab.analysis {
            tab.selection.clear();
            tab.view.dirty = true;
        }
    }

    pub(super) fn connection_status(&mut self, ui: &mut egui::Ui, tab: &mut Terminal) {
        ui.horizontal_wrapped(|ui| {
            let connected = matches!(tab.endpoint.state(), ConnectionState::Connected | ConnectionState::Connecting);
            let (state, color) = match tab.endpoint.state() {
                ConnectionState::Connected => ("Connected", theme::CONNECTED),
                ConnectionState::Connecting => ("Connecting…", theme::MUTED),
                ConnectionState::Disconnected => ("Disconnected", theme::MUTED),
                ConnectionState::Fault(_) => ("Fault", theme::ERROR),
            };
            ui.colored_label(color, state).on_hover_text(format!("{:?}", tab.endpoint.state()));
            ui.label(RichText::new(if tab.endpoint.read_only() { "Read-only replay source".into() } else { workbench_ui::framing(&tab.settings) }).small().color(theme::MUTED));
            if !tab.endpoint.read_only() && ui.selectable_label(tab.show_settings, "Settings").clicked() { tab.show_settings = !tab.show_settings; }
                if ui
                    .add(
                        theme::primary_button(if connected { "Disconnect" } else { "Reconnect" }),
                    )
                    .on_hover_text("Disconnect stops repeats and attached bridges; capture finishes. Reconnect explicitly opens this device.")
                    .clicked()
                {
                    if connected {
                        tab.stop_repeat();
                        tab.endpoint.disconnect();
                    } else if let Err(error) = tab.baud_control.validate() {
                        tab.error = Some(error.into());
                    } else {
                        tab.endpoint.disconnect();
                        match signal_forge::endpoint::open(&tab.settings, self.bus.clone()) {
                            Ok(endpoint) => {
                                tab.active_framing = SerialFraming::from(&tab.settings);
                                tab.rx_gap = true;
                                tab.lines.discard_pending();
                                tab.endpoint = endpoint;
                                tab.error = None;
                            }
                            Err(error) => tab.error = Some(error.to_string()),
                        }
                    }
                }
        });
    }

    pub(super) fn traffic_canvas(&mut self, ui: &mut egui::Ui, tab: &mut Terminal) {
        tab.refresh_view();
        let extra = match tab.tool {
            TerminalTool::Send => 0.0,
            TerminalTool::Repeat => 64.0,
            TerminalTool::Presets => 80.0,
            TerminalTool::Files => 240.0,
            TerminalTool::Capture => 240.0,
        };
        let terminal_height = (ui.available_height() - theme::SEND_AREA_HEIGHT - extra).max(40.0);
        if !ui.input(|input| input.pointer.primary_down()) {
            tab.selection.dragging = false;
        }
        let focus = egui::Id::new(("traffic-selection-focus", tab.endpoint.id().clone()));
        let rect = egui::Rect::from_min_size(
            ui.cursor().min,
            egui::vec2(ui.available_width(), terminal_height),
        );
        ui.interact(rect, focus, egui::Sense::focusable_noninteractive());
        if ui.memory(|memory| memory.has_focus(focus)) {
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
                let rows = Self::row_count(tab);
                let columns = if rows == 0 {
                    0
                } else {
                    Self::traffic_layout(tab, rows - 1).text.chars().count()
                };
                tab.selection.select_all(rows, columns);
            }
            if ui.input(|input| {
                input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Copy))
            }) {
                ui.ctx()
                    .copy_text(tab.selection.copy(Self::row_count(tab), |row| {
                        Self::traffic_layout(tab, row).text
                    }));
            }
        }
        theme::canvas_frame().show(ui, |ui| {
            egui::ScrollArea::both()
                .id_salt((tab.endpoint.id().0.clone(), "traffic"))
                .auto_shrink([false, false])
                .max_height(terminal_height)
                .min_scrolled_height(terminal_height)
                .stick_to_bottom(tab.auto_scroll)
                .show_rows(
                    ui,
                    theme::TRAFFIC_ROW_HEIGHT,
                    Self::row_count(tab),
                    |ui, range| {
                        if let Some(row) = tab.view.scroll_to.take() {
                            let rect = egui::Rect::from_min_size(
                                egui::pos2(
                                    ui.min_rect().left(),
                                    ui.max_rect().top()
                                        + (row as f32 - range.start as f32)
                                            * (theme::TRAFFIC_ROW_HEIGHT
                                                + ui.spacing().item_spacing.y),
                                ),
                                egui::vec2(1.0, theme::TRAFFIC_ROW_HEIGHT),
                            );
                            ui.scroll_to_rect(rect, Some(egui::Align::Center));
                        }
                        if tab.selection.dragging {
                            if let Some(pointer) = ui.input(|input| input.pointer.interact_pos()) {
                                let clip = ui.clip_rect();
                                let delta = if pointer.y < clip.top() {
                                    12.0
                                } else if pointer.y > clip.bottom() {
                                    -12.0
                                } else {
                                    0.0
                                };
                                if delta != 0.0 {
                                    ui.scroll_with_delta(egui::vec2(0.0, delta));
                                    ui.ctx().request_repaint();
                                }
                            }
                        }
                        for index in range {
                            Self::selectable_traffic_row(ui, tab, index);
                        }
                    },
                );
        });
    }

    fn traffic_layout(tab: &Terminal, visible: usize) -> egui::text::LayoutJob {
        let cached = tab.view.rows.get(visible).filter(|_| tab.view.ready);
        let index = cached.map_or(visible, |row| row.source);
        let (timestamp, direction, bytes, incomplete, truncated) =
            if tab.receive_mode == ReceiveMode::Line {
                let row = tab.lines.row(index).unwrap();
                (
                    row.timestamp,
                    row.direction,
                    row.bytes.as_slice(),
                    !row.complete,
                    row.truncated,
                )
            } else {
                let event = &tab.history[index];
                (
                    event.timestamp,
                    event.direction,
                    event.bytes.as_ref(),
                    false,
                    false,
                )
            };
        let color = if direction == Direction::Rx {
            theme::RX
        } else {
            theme::TX
        };
        let time = if tab.timestamps {
            let elapsed = timestamp.duration_since(UNIX_EPOCH).unwrap_or_default();
            let seconds = elapsed.as_secs() % 86400;
            format!(
                "{:02}:{:02}:{:02}.{:03} ",
                seconds / 3600,
                (seconds / 60) % 60,
                seconds % 60,
                elapsed.subsec_millis()
            )
        } else {
            String::new()
        };
        let payload = if tab.show_controls && tab.receive_mode != ReceiveMode::Hex {
            let mut text = terminal_display::control_text(bytes);
            if tab.receive_mode == ReceiveMode::Line {
                text.push_str(&terminal_display::control_text(
                    &tab.lines.row(index).unwrap().terminator,
                ));
            }
            text
        } else {
            match tab.receive_mode {
                ReceiveMode::Hex => traffic::hex(bytes),
                ReceiveMode::Line if direction == Direction::Rx => {
                    terminal_display::line_text(bytes)
                }
                _ => traffic::ascii(bytes),
            }
        };
        let chunk = if tab.receive_mode == ReceiveMode::Line {
            String::new()
        } else {
            format!(" chunk #{}", tab.history[index].sequence)
        };
        let suffix = if truncated {
            " … [display truncated]"
        } else if incomplete {
            " [partial]"
        } else {
            ""
        };
        let font = egui::FontId::monospace(12.0);
        let mut text = egui::text::LayoutJob::default();
        text.append(
            &time,
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color: theme::MUTED,
                ..Default::default()
            },
        );
        text.append(
            if !tab.analysis.direction_labels {
                ""
            } else if direction == Direction::Rx {
                "RX"
            } else {
                "TX"
            },
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color,
                ..Default::default()
            },
        );
        text.append(
            &chunk,
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color: theme::MUTED,
                ..Default::default()
            },
        );
        text.append(
            &format!(
                "{}{payload}{suffix}",
                if tab.analysis.direction_labels || !chunk.is_empty() {
                    "  "
                } else {
                    ""
                }
            ),
            0.0,
            egui::TextFormat {
                font_id: font,
                color,
                ..Default::default()
            },
        );
        if let Some(row) = cached {
            use signal_forge::traffic_analysis::duration_text;
            let mut annotations = Vec::new();
            if tab.analysis.delta_displayed {
                if let Some(delta) = row.timing.displayed {
                    annotations.push(format!("Δ {}", duration_text(delta)));
                }
            }
            if tab.analysis.delta_rx {
                if let Some(delta) = row.timing.rx {
                    annotations.push(format!("ΔRX {}", duration_text(delta)));
                }
            }
            if tab.analysis.delta_tx {
                if let Some(delta) = row.timing.tx {
                    annotations.push(format!("ΔTX {}", duration_text(delta)));
                }
            }
            if tab.analysis.line_span {
                if let Some(span) = row.span {
                    annotations.push(format!("RX span {}", duration_text(span)));
                }
            }
            if tab.analysis.response_latency {
                if let Some((duration, sequence)) = row.response {
                    annotations.push(format!(
                        "TX #{sequence} → RX {} (observed)",
                        duration_text(duration)
                    ));
                }
            }
            if !annotations.is_empty() {
                text.append(
                    &format!("  [{}]", annotations.join(" · ")),
                    0.0,
                    egui::TextFormat {
                        font_id: egui::FontId::monospace(12.0),
                        color: theme::MUTED,
                        ..Default::default()
                    },
                );
            }
        }
        text
    }

    fn row_count(tab: &Terminal) -> usize {
        if tab.view.ready {
            return tab.view.rows.len();
        }
        if tab.receive_mode == ReceiveMode::Line {
            tab.lines.len()
        } else {
            tab.history.len()
        }
    }

    #[cfg(test)]
    fn traffic_row(ui: &mut egui::Ui, tab: &Terminal, index: usize) {
        let response = ui.add(
            egui::Label::new(Self::traffic_layout(tab, index))
                .wrap_mode(egui::TextWrapMode::Extend),
        );
        if tab.receive_mode == ReceiveMode::Line {
            if let Some(timing) = tab.lines.row(index).and_then(|row| row.timing.as_ref()) {
                response.on_hover_text(timing.tooltip());
            }
        }
    }

    fn selectable_traffic_row(ui: &mut egui::Ui, tab: &mut Terminal, index: usize) {
        use signal_forge::terminal_selection::Position;
        let source = tab.view.rows.get(index).map_or(index, |row| row.source);
        let (pos, galley, response) = egui::Label::new(Self::traffic_layout(tab, index))
            .wrap_mode(egui::TextWrapMode::Extend)
            .sense(egui::Sense::click_and_drag())
            .layout_in_ui(ui);
        if response.is_pointer_button_down_on() && ui.input(|input| input.pointer.primary_pressed())
        {
            let focus = egui::Id::new(("traffic-selection-focus", tab.endpoint.id().clone()));
            ui.memory_mut(|memory| memory.request_focus(focus));
            if let Some(pointer) = ui.input(|input| input.pointer.interact_pos()) {
                let point = Position {
                    row: index,
                    column: galley.cursor_from_pos(pointer - pos).ccursor.index,
                };
                if !tab.selection.dragging {
                    tab.selection.anchor = Some(point);
                    tab.selection.dragging = true;
                    tab.auto_scroll = false;
                }
                tab.selection.head = Some(point);
            }
        }
        if tab.selection.dragging {
            if let Some(pointer) = ui.input(|input| input.pointer.interact_pos()) {
                let clip = ui.clip_rect();
                let y = pointer.y.clamp(clip.top() + 1.0, clip.bottom() - 1.0);
                if y >= response.rect.top() && y <= response.rect.bottom() {
                    tab.selection.head = Some(Position {
                        row: index,
                        column: galley
                            .cursor_from_pos(egui::pos2(pointer.x, y) - pos)
                            .ccursor
                            .index,
                    });
                }
            }
        }
        if let Some(row) = tab.view.rows.get(index) {
            let color = if tab.view.current == Some(index) {
                Some(egui::Color32::from_rgb(45, 85, 120))
            } else if row.matched {
                Some(egui::Color32::from_rgb(85, 70, 25))
            } else {
                row.emphasis.as_ref().map(|(emphasis, _)| match emphasis {
                    signal_forge::traffic_analysis::Emphasis::Error => {
                        egui::Color32::from_rgb(90, 30, 35)
                    }
                    signal_forge::traffic_analysis::Emphasis::Warning => {
                        egui::Color32::from_rgb(75, 60, 25)
                    }
                    signal_forge::traffic_analysis::Emphasis::Ready => {
                        egui::Color32::from_rgb(25, 70, 45)
                    }
                })
            };
            if let Some(color) = color {
                ui.painter().rect_filled(response.rect, 0.0, color);
            }
        }
        if let Some(range) = tab.selection.columns(index, galley.text().chars().count()) {
            if !range.is_empty() {
                let left = pos
                    + galley
                        .pos_from_ccursor(egui::text::CCursor::new(range.start))
                        .left_top()
                        .to_vec2();
                let right = pos
                    + galley
                        .pos_from_ccursor(egui::text::CCursor::new(range.end))
                        .right_bottom()
                        .to_vec2();
                ui.painter().rect_filled(
                    egui::Rect::from_min_max(left, right),
                    0.0,
                    ui.visuals().selection.bg_fill,
                );
            }
        }
        ui.painter().galley(pos, galley, theme::RX);
        if let Some((_, label)) = tab
            .view
            .rows
            .get(index)
            .and_then(|row| row.emphasis.as_ref())
        {
            response
                .clone()
                .on_hover_text(format!("Highlight: {label}"));
        }
        if tab.receive_mode == ReceiveMode::Line {
            if let Some(timing) = tab.lines.row(source).and_then(|row| row.timing.as_ref()) {
                response.on_hover_text(timing.tooltip());
            }
        }
    }

    pub(super) fn tool_strip(&mut self, ui: &mut egui::Ui, tab: &mut Terminal) {
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut tab.tool, TerminalTool::Send, "Send");
            let active = tab.repeat.as_ref().is_some_and(|handle| handle.is_active());
            ui.selectable_value(
                &mut tab.tool,
                TerminalTool::Repeat,
                if active { "Repeat (active)" } else { "Repeat" },
            );
            ui.selectable_value(&mut tab.tool, TerminalTool::Presets, "Presets");
            ui.selectable_value(&mut tab.tool, TerminalTool::Files, "Files / RX");
            ui.selectable_value(&mut tab.tool, TerminalTool::Capture, "Triggered capture");
            if active && ui.add(theme::danger_button("Stop")).clicked() {
                tab.stop_repeat();
            }
        });
    }
    pub(super) fn preset_panel(&mut self, ui: &mut egui::Ui, tab: &mut Terminal) {
        egui::ScrollArea::vertical()
            .id_salt((tab.endpoint.id().0.clone(), "terminal-presets"))
            .max_height(65.0)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for preset in self.presets {
                        if ui
                            .button(&preset.name)
                            .on_hover_text(format!("{}\n{}", preset.description, preset.shortcut))
                            .clicked()
                        {
                            *self.preset_request =
                                Some((tab.endpoint.id().clone(), preset.clone()));
                        }
                    }
                    if self.presets.is_empty() {
                        ui.weak("No presets in this profile");
                    }
                    if ui.small_button("Manage…").clicked() {
                        *self.preset_library_open = true;
                    }
                });
            });
    }

    pub(super) fn send_panel(&mut self, ui: &mut egui::Ui, tab: &mut Terminal) {
        if tab.endpoint.read_only() {
            Self::playback_panel(ui, tab);
            return;
        }
        ui.horizontal(|ui| {
            // A global endpoint-based ID stays stable as virtualized traffic rows
            // are added above this editor or its terminal is moved in the dock.
            let input_id = egui::Id::new(("terminal-payload", tab.endpoint.id().0.clone()));
            let focused = ui.memory(|memory| memory.has_focus(input_id));
            // Consume these keys before TextEdit: Enter otherwise surrenders focus,
            // and arrows would move the caret instead of recalling a command.
            let (enter, older, newer) = ui.input_mut(|input| {
                if !focused {
                    return (false, false, false);
                }
                (
                    input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                    input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                    input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                )
            });
            let input = ui.add(
                egui::TextEdit::singleline(&mut tab.input)
                    .id(input_id)
                    .desired_width((ui.available_width() - 60.0).max(60.0))
                    .hint_text("Payload · Enter sends · Up recalls"),
            );
            // Process text events before recall so a draft includes every character
            // entered in this frame. The navigation key itself was consumed above.
            if input.changed() {
                tab.send_history.edited();
            }
            let focused = input.has_focus();
            if older && focused {
                tab.history_older();
            }
            if newer && focused {
                tab.history_newer();
            }
            let clicked = ui
                .add_enabled(
                    tab.endpoint.state() == ConnectionState::Connected,
                    theme::primary_button("Send"),
                )
                .clicked();
            if clicked || (enter && focused) {
                tab.send();
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut tab.encoding, Encoding::Text, "Text");
            ui.selectable_value(&mut tab.encoding, Encoding::Hex, "Hex bytes");
            egui::ComboBox::from_id_salt((tab.endpoint.id().0.clone(), "ending"))
                .selected_text(format!("Ending: {:?}", tab.ending))
                .show_ui(ui, |ui| {
                    for (value, label) in [
                        (LineEnding::None, "None"),
                        (LineEnding::Cr, "CR"),
                        (LineEnding::Lf, "LF"),
                        (LineEnding::CrLf, "CRLF"),
                    ] {
                        ui.selectable_value(&mut tab.ending, value, label);
                    }
                });
            ui.add_enabled(
                tab.encoding == Encoding::Text,
                egui::Checkbox::new(&mut tab.escapes, "Interpret escapes"),
            );
        });
    }

    pub(super) fn repeat_panel(&mut self, ui: &mut egui::Ui, tab: &mut Terminal) {
        ui.horizontal_wrapped(|ui| {
            let active = tab.repeat.as_ref().is_some_and(|handle| handle.is_active());
            ui.label("Repeat every");
            ui.add_enabled(
                !active,
                egui::DragValue::new(&mut tab.repeat_interval_ms)
                    .range(1..=86400000)
                    .suffix(" ms"),
            );
            ui.add_enabled(
                !active,
                egui::Checkbox::new(&mut tab.continuous, "Until stopped"),
            );
            if !tab.continuous {
                ui.label("Count");
                ui.add_enabled(
                    !active,
                    egui::DragValue::new(&mut tab.repeat_count).range(1..=1000000),
                );
            }
            if ui
                .add_enabled(
                    !active
                        && tab.endpoint.state() == ConnectionState::Connected
                        && !tab.endpoint.read_only(),
                    egui::Button::new("Start repeat"),
                )
                .clicked()
            {
                tab.start_repeat();
            }
            if ui
                .add_enabled(active, theme::danger_button("Stop repeat"))
                .clicked()
            {
                tab.stop_repeat();
            }
        });
        if let Some(handle) = &tab.repeat {
            let status = handle.status();
            let total = status
                .count
                .map(|count| count.to_string())
                .unwrap_or_else(|| "continuous".into());
            ui.label(
                RichText::new(format!(
                    "Repeat: {:?} · {} / {} fully sent",
                    status.state, status.sent, total
                ))
                .color(if handle.is_active() {
                    theme::ACCENT
                } else {
                    theme::MUTED
                }),
            );
        }
    }

    fn open_file_dialog(tab: &mut Terminal, recording: bool) {
        let path = std::path::Path::new(if recording {
            &tab.recording_path
        } else {
            &tab.file_path
        });
        let mut dialog = egui_file_dialog::FileDialog::new()
            .id(egui::Id::new((
                "terminal-file-dialog",
                tab.endpoint.id().clone(),
            )))
            .title(if recording {
                "Choose RX log destination"
            } else {
                "Choose file to send"
            });
        if path.is_dir() {
            dialog = dialog.initial_directory(path.to_path_buf());
        } else {
            if let Some(parent) = path.parent().filter(|parent| parent.is_dir()) {
                dialog = dialog.initial_directory(parent.to_path_buf());
            }
            if recording {
                if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                    dialog = dialog.default_file_name(name);
                }
            }
        }
        if recording {
            dialog.save_file();
        } else {
            dialog.pick_file();
        }
        tab.file_dialog = Some((dialog, recording));
    }

    pub(super) fn files_panel(&mut self, ui: &mut egui::Ui, tab: &mut Terminal) {
        use signal_forge::file_transfer::{FileMode, TransferState};
        ui.label("Send file");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut tab.file_path).desired_width(160.0));
            if ui.button("Browse…").clicked() {
                Self::open_file_dialog(tab, false);
            }
            if ui
                .add_enabled(
                    tab.endpoint.state() == ConnectionState::Connected
                        && !tab.endpoint.read_only()
                        && !tab.endpoint.file_send_active(),
                    egui::Button::new("Send file"),
                )
                .clicked()
            {
                match tab.endpoint.send_file_mode(
                    std::path::Path::new(&tab.file_path),
                    tab.file_mode,
                    tab.file_chunk,
                    Duration::from_millis(tab.file_delay_ms),
                ) {
                    Ok(handle) => {
                        tab.file_handle = Some(handle);
                        tab.error = None;
                    }
                    Err(error) => tab.error = Some(error.to_string()),
                }
            }
        });
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(("file-send-mode", tab.endpoint.id().clone()))
                .selected_text(format!("{:?}", tab.file_mode))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut tab.file_mode, FileMode::Raw, "Raw / Binary");
                    ui.selectable_value(
                        &mut tab.file_mode,
                        FileMode::Ascii,
                        "ASCII / Text (UTF-8)",
                    );
                    ui.selectable_value(&mut tab.file_mode, FileMode::Hex, "Hex");
                });
            ui.label("Chunk");
            ui.add(
                egui::DragValue::new(&mut tab.file_chunk)
                    .range(1..=65536)
                    .suffix(" B"),
            );
            ui.label("Delay");
            ui.add(
                egui::DragValue::new(&mut tab.file_delay_ms)
                    .range(0..=60000)
                    .suffix(" ms"),
            );
        });
        ui.weak("Raw/text preserve line endings; Hex is validated before sending.");
        if let Some(handle) = &tab.file_handle {
            let status = handle.status();
            if status.state == TransferState::Preparing {
                ui.label("Preparing and validating file…");
            }
            let ratio = if status.total == 0 {
                if status.state == TransferState::Completed {
                    1.0
                } else {
                    0.0
                }
            } else {
                status.sent as f32 / status.total as f32
            };
            ui.add(egui::ProgressBar::new(ratio).text(format!(
                "{:?}: {} / {} bytes",
                status.state, status.sent, status.total
            )));
            if ui
                .add_enabled(handle.is_active(), egui::Button::new("Cancel file send"))
                .clicked()
            {
                handle.cancel();
            }
        }
        ui.separator();
        ui.label("Record RX to a new file");
        egui::ComboBox::from_id_salt(("rx-file-mode", tab.endpoint.id().clone()))
            .selected_text(format!("{:?}", tab.recording_mode))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut tab.recording_mode, FileMode::Raw, "Raw / Binary");
                ui.selectable_value(
                    &mut tab.recording_mode,
                    FileMode::Ascii,
                    "ASCII / Text (escaped binary)",
                );
                ui.selectable_value(&mut tab.recording_mode, FileMode::Hex, "Hex");
            });
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut tab.recording_path).desired_width(160.0));
            if ui.button("Save as…").clicked() {
                Self::open_file_dialog(tab, true);
            }
            if ui
                .add_enabled(
                    tab.endpoint.state() == ConnectionState::Connected
                        && !tab
                            .recording
                            .as_ref()
                            .is_some_and(|recording| recording.is_active()),
                    egui::Button::new("Record RX"),
                )
                .clicked()
            {
                if let Some(recording) = &mut tab.recording {
                    recording.finish();
                }
                match signal_forge::raw_recording::RawRecording::start_mode(
                    std::path::Path::new(&tab.recording_path),
                    tab.endpoint.id().clone(),
                    self.bus,
                    tab.recording_mode,
                ) {
                    Ok(recording) => {
                        tab.recording = Some(recording);
                        tab.error = None;
                    }
                    Err(error) => tab.error = Some(error),
                }
            }
            if ui.button("Stop recording").clicked() {
                if let Some(recording) = &tab.recording {
                    recording.stop();
                }
            }
        });
        if let Some(recording) = &tab.recording {
            ui.label(recording.status());
        }
    }

    pub(super) fn status_footer(&mut self, ui: &mut egui::Ui, tab: &mut Terminal) {
        if let ConnectionState::Fault(error) = tab.endpoint.state() {
            ui.colored_label(theme::ERROR, error);
        }
        if let Some(error) = &tab.error {
            ui.colored_label(theme::ERROR, format!("{}: {error}", tab.settings.path));
        }
    }
}

#[cfg(test)]
mod timing_hover_tests {
    use super::*;

    fn rendered_text(shapes: &[egui::epaint::ClippedShape]) -> String {
        fn append(shape: &egui::epaint::Shape, text: &mut String) {
            match shape {
                egui::epaint::Shape::Text(shape) => {
                    text.push_str(shape.galley.text());
                    text.push('\n');
                }
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        append(shape, text);
                    }
                }
                _ => {}
            }
        }
        let mut text = String::new();
        for shape in shapes {
            append(&shape.shape, &mut text);
        }
        text
    }
    #[test]
    fn retained_canvas_select_all_and_copy_include_offscreen_rows() {
        let mut tab = Terminal::restored(&signal_forge::workspace::SavedTerminal {
            settings: SerialSettings {
                path: "/dev/test".into(),
                ..Default::default()
            },
            timestamps: false,
            auto_scroll: false,
            ..Default::default()
        });
        tab.receive(Arc::new(TrafficEvent {
            sequence: 1,
            timestamp: UNIX_EPOCH,
            endpoint: tab.endpoint.id().clone(),
            direction: Direction::Rx,
            bytes: Arc::from(
                (0..300)
                    .map(|i| format!("row {i}\r\n"))
                    .collect::<String>()
                    .into_bytes(),
            ),
        }));
        let ctx = egui::Context::default();
        let bus = TrafficBus::default();
        let mut selected = None;
        let mut known = Vec::new();
        let mut request = None;
        let mut library = false;
        let mut render = |events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(500.0, 260.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        TerminalViewer {
                            bus: &bus,
                            selected: &mut selected,
                            known_ports: &mut known,
                            presets: &[],
                            preset_request: &mut request,
                            preset_library_open: &mut library,
                        }
                        .traffic_canvas(ui, &mut tab);
                    });
                },
            )
        };
        let _ = render(vec![]);
        let point = egui::pos2(30.0, 25.0);
        let _ = render(vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ]);
        let _ = render(vec![egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }]);
        let _ = render(vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }]);
        let copied = render(vec![egui::Event::Copy]);
        let text = copied
            .platform_output
            .commands
            .iter()
            .find_map(|command| {
                if let egui::OutputCommand::CopyText(text) = command {
                    Some(text)
                } else {
                    None
                }
            })
            .expect("Focused canvas must copy retained selection");
        assert!(text.starts_with("RX  row 0\n"), "{text}");
        assert!(text.ends_with("RX  row 299"), "{text}");
        assert_eq!(text.lines().count(), 300);
        let _ = render(vec![
            egui::Event::PointerMoved(point),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -600.0),
                modifiers: Default::default(),
            },
        ]);
        for _ in 0..30 {
            let _ = render(vec![]);
        }
        let after_scroll = render(vec![egui::Event::Copy]);
        let text = after_scroll
            .platform_output
            .commands
            .iter()
            .find_map(|command| {
                if let egui::OutputCommand::CopyText(text) = command {
                    Some(text)
                } else {
                    None
                }
            })
            .expect("Selection focus must survive scrolling its original rows offscreen");
        assert_eq!(text.lines().count(), 300);
        let _ = render(vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ]);
        let _ = render(vec![egui::Event::PointerMoved(egui::pos2(100.0, 500.0))]);
        for _ in 0..120 {
            let _ = render(vec![]);
        }
        let _ = render(vec![egui::Event::PointerButton {
            pos: egui::pos2(100.0, 500.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }]);
        let dragged = render(vec![egui::Event::Copy]);
        let text = dragged
            .platform_output
            .commands
            .iter()
            .find_map(|command| {
                if let egui::OutputCommand::CopyText(text) = command {
                    Some(text)
                } else {
                    None
                }
            })
            .expect("Drag selection must remain copyable outside the viewport");
        assert!(
            text.lines().count() > 25,
            "Drag must scroll through more than one viewport: {} rows",
            text.lines().count()
        );
    }

    #[test]
    fn filters_labels_and_timing_render_without_mutating_retained_history() {
        use signal_forge::traffic_analysis::{ViewSettings, Visibility};
        let mut tab = Terminal::restored(&signal_forge::workspace::SavedTerminal {
            settings: SerialSettings {
                path: "/dev/test".into(),
                ..Default::default()
            },
            timestamps: false,
            ..Default::default()
        });
        for (sequence, direction, bytes) in [
            (1, Direction::Rx, b"START\n".as_slice()),
            (2, Direction::Tx, b"STATUS?".as_slice()),
            (3, Direction::Rx, b"READY\n".as_slice()),
        ] {
            tab.receive(Arc::new(TrafficEvent {
                sequence,
                timestamp: UNIX_EPOCH + Duration::from_millis(sequence * 10),
                endpoint: tab.endpoint.id().clone(),
                direction,
                bytes: Arc::from(bytes),
            }));
        }
        tab.analysis = ViewSettings {
            visibility: Visibility::Rx,
            direction_labels: false,
            delta_displayed: true,
            delta_tx: true,
            response_latency: true,
            ..Default::default()
        };
        tab.refresh_view();
        assert_eq!(TerminalViewer::row_count(&tab), 2);
        let row = TerminalViewer::traffic_layout(&tab, 1).text;
        assert!(row.starts_with("READY  ["), "{row}");
        assert!(
            row.contains("Δ 20.00 ms") && row.contains("ΔTX 10.00 ms"),
            "{row}"
        );
        assert!(row.contains("TX #2 → RX 10.00 ms (observed)"), "{row}");
        tab.analysis.visibility = Visibility::Tx;
        tab.refresh_view();
        assert!(TerminalViewer::traffic_layout(&tab, 0)
            .text
            .starts_with("STATUS?"));
        assert_eq!(tab.history.len(), 3);
        let restored = Terminal::restored(&tab.saved());
        assert_eq!(restored.analysis, tab.analysis);
    }

    #[test]
    fn search_navigation_scrolls_to_an_offscreen_match() {
        use signal_forge::traffic_analysis::{Pattern, PatternMode};
        let mut tab = Terminal::restored(&signal_forge::workspace::SavedTerminal {
            settings: SerialSettings {
                path: "/dev/test".into(),
                ..Default::default()
            },
            timestamps: false,
            auto_scroll: false,
            ..Default::default()
        });
        tab.receive(Arc::new(TrafficEvent {
            sequence: 1,
            timestamp: UNIX_EPOCH,
            endpoint: tab.endpoint.id().clone(),
            direction: Direction::Rx,
            bytes: Arc::from(
                (0..300)
                    .map(|i| format!("row {i}\n"))
                    .collect::<String>()
                    .into_bytes(),
            ),
        }));
        tab.analysis.search = Pattern {
            mode: PatternMode::Regex,
            value: "^row 299".into(),
        };
        let ctx = egui::Context::default();
        let bus = TrafficBus::default();
        let mut selected = None;
        let mut known = Vec::new();
        let mut request = None;
        let mut library = false;
        let mut render = |tab: &mut Terminal, time| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(500.0, 260.0),
                    )),
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        TerminalViewer {
                            bus: &bus,
                            selected: &mut selected,
                            known_ports: &mut known,
                            presets: &[],
                            preset_request: &mut request,
                            preset_library_open: &mut library,
                        }
                        .traffic_canvas(ui, tab);
                    });
                },
            )
        };
        let first = render(&mut tab, 0.0);
        assert!(!rendered_text(&first.shapes).contains("row 299"));
        tab.navigate_match(true);
        let mut output = render(&mut tab, 0.1);
        for frame in 2..30 {
            output = render(&mut tab, frame as f64 * 0.1);
        }
        assert!(
            rendered_text(&output.shapes).contains("row 299"),
            "{}",
            rendered_text(&output.shapes)
        );
        assert_eq!(tab.view.current, Some(299));
    }

    #[test]
    fn timing_is_hover_only_and_only_on_rx_line_rows() {
        let mut tab = Terminal::restored(&signal_forge::workspace::SavedTerminal {
            settings: SerialSettings {
                path: "/dev/test".into(),
                ..Default::default()
            },
            ..Default::default()
        });
        tab.receive(Arc::new(TrafficEvent {
            sequence: 1,
            timestamp: UNIX_EPOCH,
            endpoint: tab.endpoint.id().clone(),
            direction: Direction::Rx,
            bytes: Arc::from(b"HELLO\n".as_slice()),
        }));
        let ctx = egui::Context::default();
        ctx.style_mut(|style| style.interaction.tooltip_delay = 0.0);
        let render = |time, position| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 400.0),
                    )),
                    time: Some(time),
                    events: vec![egui::Event::PointerMoved(position)],
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        TerminalViewer::traffic_row(ui, &tab, 0);
                    });
                },
            )
        };
        let normal = render(0.0, egui::pos2(400.0, 300.0));
        assert!(rendered_text(&normal.shapes).contains("HELLO"));
        assert!(!rendered_text(&normal.shapes).contains("Observed RX span"));
        // Egui resolves hover from the preceding frame's widget rectangles.
        let _ = render(1.0, egui::pos2(20.0, 15.0));
        let hover = render(2.0, egui::pos2(20.0, 15.0));
        let text = rendered_text(&hover.shapes);
        assert!(
            text.contains("Observed RX span: 0.0 ms / single read"),
            "{text}"
        );
        assert!(text.contains("Calculated wire time: 3.1 ms"), "{text}");
        assert!(text.contains("19200 baud · 8N1"), "{text}");
        tab.receive(Arc::new(TrafficEvent {
            sequence: 2,
            timestamp: UNIX_EPOCH,
            endpoint: tab.endpoint.id().clone(),
            direction: Direction::Tx,
            bytes: Arc::from(b"request".as_slice()),
        }));
        // Reuse the hovered location for a TX row: it must have no RX tooltip.
        for time in [2.1, 2.2] {
            let output = ctx.run(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        TerminalViewer::traffic_row(ui, &tab, 1);
                    });
                },
            );
            assert!(!rendered_text(&output.shapes).contains("Observed RX span"));
        }
        for (index, mode) in [ReceiveMode::RawChunks, ReceiveMode::Hex]
            .into_iter()
            .enumerate()
        {
            tab.receive_mode = mode;
            let _ = ctx.run(
                egui::RawInput {
                    time: Some(3.0 + index as f64 * 2.0),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        TerminalViewer::traffic_row(ui, &tab, 0);
                    });
                },
            );
            let output = ctx.run(
                egui::RawInput {
                    time: Some(4.0 + index as f64 * 2.0),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        TerminalViewer::traffic_row(ui, &tab, 0);
                    });
                },
            );
            assert!(!rendered_text(&output.shapes).contains("Observed RX span"));
        }
    }
}
