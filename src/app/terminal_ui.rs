//! Terminal presentation sections; transport actions remain on the existing endpoints.
use super::*;

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
                let connected = tab.endpoint.state() == ConnectionState::Connected;
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
                        match SerialEndpoint::open(&tab.settings, self.bus.clone()) {
                            Ok(endpoint) => {
                                tab.endpoint = Box::new(endpoint);
                                tab.error = None;
                            }
                            Err(error) => tab.error = Some(error.to_string()),
                        }
                    }
                }
            });
        });
    }

    pub(super) fn display_controls(&mut self, ui: &mut egui::Ui, tab: &mut Terminal) {
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut tab.receive_mode, ReceiveMode::Line, "Line");
            ui.selectable_value(&mut tab.receive_mode, ReceiveMode::RawChunks, "Raw Chunks");
            ui.selectable_value(&mut tab.receive_mode, ReceiveMode::Hex, "Hex");
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
                tab.lines.clear();
                for event in &tab.history {
                    if tab.rx_breaks.contains(&event.sequence) {
                        tab.lines.discard_pending();
                    }
                    tab.lines.receive(event);
                }
            }
            ui.checkbox(&mut tab.timestamps, "Timestamps");
            ui.checkbox(&mut tab.auto_scroll, "Auto-scroll");
            ui.checkbox(&mut tab.paused, "Pause display");
            if ui.button("Clear").clicked() {
                tab.history.clear();
                tab.lines.clear();
                tab.rx_breaks.clear();
                tab.rx_gap = false;
            }
        });
    }

    pub(super) fn connection_status(&mut self, ui: &mut egui::Ui, tab: &mut Terminal) {
        match tab.endpoint.state() {
            ConnectionState::Connected => {
                ui.label(
                    RichText::new(format!(
                        "Connected  ·  RX {} B  ·  TX {} B",
                        tab.rx_bytes, tab.tx_bytes
                    ))
                    .color(theme::CONNECTED),
                );
            }
            ConnectionState::Disconnected => {
                ui.colored_label(
                    theme::WARNING,
                    "Disconnected — adjust settings above and reconnect",
                );
            }
            ConnectionState::Fault(error) => {
                ui.colored_label(
                    theme::ERROR,
                    format!("{}: {error}", tab.endpoint.display_name()),
                );
            }
        }
        if tab.paused {
            ui.label("Display paused; new rows are discarded while serial I/O continues.");
        }
    }

    pub(super) fn traffic_canvas(&mut self, ui: &mut egui::Ui, tab: &mut Terminal) {
        let terminal_height = (ui.available_height() - theme::SEND_AREA_HEIGHT).max(80.0);
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
                    if tab.receive_mode == ReceiveMode::Line {
                        tab.lines.len()
                    } else {
                        tab.history.len()
                    },
                    |ui, range| {
                        for index in range {
                            Self::traffic_row(ui, tab, index);
                        }
                    },
                );
        });
    }

    fn traffic_row(ui: &mut egui::Ui, tab: &Terminal, index: usize) {
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
        let payload = match tab.receive_mode {
            ReceiveMode::Hex => traffic::hex(bytes),
            ReceiveMode::Line if direction == Direction::Rx => terminal_display::line_text(bytes),
            _ => traffic::ascii(bytes),
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
        ui.add(
            egui::Label::new(
                RichText::new(format!(
                    "{time}{}{chunk}  {payload}{suffix}",
                    if direction == Direction::Rx {
                        "RX"
                    } else {
                        "TX"
                    }
                ))
                .monospace()
                .color(color),
            )
            .wrap_mode(egui::TextWrapMode::Extend),
        );
    }

    pub(super) fn send_panel(&mut self, ui: &mut egui::Ui, tab: &mut Terminal) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut tab.encoding, Encoding::Text, "Text");
            ui.selectable_value(&mut tab.encoding, Encoding::Hex, "Hex bytes");
            ui.add_enabled(
                tab.encoding == Encoding::Text,
                egui::Checkbox::new(&mut tab.escapes, "Interpret escapes"),
            );
        });
        ui.horizontal(|ui| {
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
                    .desired_width((ui.available_width() - 70.0).max(100.0))
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
                    !active && tab.endpoint.state() == ConnectionState::Connected,
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

    pub(super) fn status_footer(&mut self, ui: &mut egui::Ui, tab: &mut Terminal) {
        if let Some(error) = &tab.error {
            ui.colored_label(theme::ERROR, format!("{}: {error}", tab.settings.path));
        }
        ui.small(
            "Enter sends · Up cycles previous messages · Down returns to draft · History is per terminal.",
        );
    }
}
