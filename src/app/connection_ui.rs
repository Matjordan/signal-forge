use super::*;
use signal_forge::{bridge::Bridge, virtual_pair::VirtualPair};
use signal_forge::{
    capture::{Capture, CaptureState},
    inspector::{timestamp_utc, DirectionFilter, Inspector},
};
use std::path::Path;

pub(super) struct BridgeView {
    pub(super) bridge: Bridge,
    events: Receiver<Arc<TrafficEvent>>,
    inspector: Inspector,
    capture_path: String,
    pub(super) pending_artifact: Option<std::path::PathBuf>,
    capture: Option<Capture>,
    capture_error: Option<String>,
}
impl Workbench {
    pub(super) fn create_pair(&mut self) {
        if self.pairs.iter().any(|p| p.name == self.pair_name) {
            self.error = Some(format!("PTY pair {} already exists", self.pair_name));
        } else {
            let directory = if self.pair_directory.trim().is_empty() {
                None
            } else {
                Some(Path::new(&self.pair_directory))
            };
            let timing = if self.pair_emulated {
                signal_forge::virtual_pair::LinkTiming::Emulated(SerialFraming::from(
                    &self.pair_framing,
                ))
            } else {
                signal_forge::virtual_pair::LinkTiming::Unlimited
            };
            match VirtualPair::create_with_timing(&self.pair_name, directory, timing) {
                Ok(pair) => {
                    log::info!(
                        "Created PTY pair {}: {} <-> {} · {}",
                        pair.name,
                        pair.paths[0],
                        pair.paths[1],
                        pair.timing.label()
                    );
                    self.pairs.push(pair);
                    self.error = None;
                }
                Err(error) => self.error = Some(format!("PTY pair {}: {error}", self.pair_name)),
            }
        }
    }
    pub(super) fn start_bridge(&mut self) {
        let ports: Result<Vec<_>, _> = [&self.bridge_a, &self.bridge_b]
            .iter()
            .map(|selection| {
                self.dock
                    .iter_all_tabs()
                    .find(|(_, t)| Some(t.endpoint.id()) == selection.as_ref())
                    .ok_or(EndpointError::Disconnected)?
                    .1
                    .endpoint
                    .bridge_port()
            })
            .collect();
        match ports.and_then(|ports| Bridge::start(ports[0].clone(), ports[1].clone())) {
            Ok(bridge) => {
                let events = bridge.subscribe(512);
                self.bridges.push(BridgeView {
                    bridge,
                    pending_artifact: None,
                    events,
                    inspector: Inspector::new(),
                    capture_path: self
                        .session
                        .as_ref()
                        .map(|session| {
                            session
                                .suggested_path(
                                    &format!("bridge-{}", self.bridges.len() + 1),
                                    "jsonl",
                                )
                                .display()
                                .to_string()
                        })
                        .unwrap_or_else(|| {
                            format!(
                                "bridge-{}.jsonl",
                                signal_forge::inspector::timestamp_ns(std::time::SystemTime::now())
                            )
                        }),
                    capture: None,
                    capture_error: None,
                });
                self.error = None;
            }
            Err(error) => {
                self.error = Some(format!(
                    "Bridge {} <-> {}: {error}",
                    self.bridge_a
                        .as_ref()
                        .map(|id| id.0.as_str())
                        .unwrap_or("A not selected"),
                    self.bridge_b
                        .as_ref()
                        .map(|id| id.0.as_str())
                        .unwrap_or("B not selected")
                ))
            }
        }
    }
    pub(super) fn connection_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Q)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.setup.is_some() || self.replay_setup.is_some() || self.keyboard_editing(ctx) {
            return;
        }
        let modifiers = egui::Modifiers::CTRL | egui::Modifiers::SHIFT;
        if ctx.input_mut(|i| i.consume_key(modifiers, egui::Key::N)) {
            self.open_setup(workbench_ui::SetupKind::Pair);
        }
        if ctx.input_mut(|i| i.consume_key(modifiers, egui::Key::B)) {
            self.open_setup(workbench_ui::SetupKind::Bridge);
        }
        if ctx.input_mut(|i| i.consume_key(modifiers, egui::Key::R)) {
            if let Some(view) = self.bridges.first_mut() {
                view.toggle_capture();
            }
        }
        if ctx.input_mut(|i| i.consume_key(modifiers, egui::Key::M)) {
            for view in &mut self.bridges {
                view.inspector.paused = !view.inspector.paused;
            }
        }
    }
    pub(super) fn connections_ui(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.heading("Virtual pairs");
        if ui.small_button("+ Virtual pair").clicked() {
            self.open_setup(workbench_ui::SetupKind::Pair);
        }
        let mut open = None;
        let mut remove = None;
        for (index, pair) in self.pairs.iter().enumerate() {
            ui.push_id(index, |ui| {
                ui.label(format!("{} · {:?}", pair.name, pair.state()));
                ui.small(pair.timing.label());
                for (side, path) in pair.paths.iter().enumerate() {
                    ui.small(format!("{}: {path}", if side == 0 { "A" } else { "B" }));
                    let owned = self.dock.iter_all_tabs().any(|(_, tab)| {
                        [path, &pair.raw_paths[side]].contains(&&tab.settings.path)
                            && matches!(
                                tab.endpoint.state(),
                                ConnectionState::Connected | ConnectionState::Connecting
                            )
                    });
                    ui.small(if owned {
                        "In use by Signal Forge; external apps should use the peer"
                    } else {
                        "Available to open in Signal Forge or an external app"
                    });
                    ui.horizontal_wrapped(|ui| {
                        let response = ui.small_button(format!(
                            "Open {}",
                            if side == 0 { "A" } else { "B" }
                        ));
                        let center = response.rect.center() * ui.ctx().pixels_per_point();
                        log::debug!("PTY open control {path}: {},{}", center.x, center.y);
                        if response.clicked() {
                            open = Some(path.clone());
                        }
                        if ui.small_button("Copy path").clicked() {
                            ui.ctx().copy_text(path.clone());
                        }
                        if ui.add_enabled(!owned, egui::Button::new("Release stale exclusive flag")).on_hover_text("Use only after the external client exits. This releases TIOCEXCL without sudo; it does not override advisory locks. Releasing a live client can permit conflicting opens.").clicked() {
                            if let Err(error)=pair.release_exclusive(side) { self.error=Some(error); }
                            self.pair_diagnostics=pair.diagnostics();
                        }
                    });
                }
                if ui.small_button("Check paths/access").clicked() {
                    self.pair_diagnostics = pair.diagnostics();
                }
                for diagnostic in self
                    .pair_diagnostics
                    .iter()
                    .filter(|diagnostic| pair.paths.contains(&diagnostic.path))
                {
                    ui.small(format!("{} → {}", diagnostic.path, diagnostic.target));
                    ui.small(&diagnostic.permissions);
                    ui.small(&diagnostic.access);
                }
                if ui.small_button("Remove pair").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(path) = open {
            // PTY settings are separate from shared link emulation. Do not inherit
            // hardware/software flow control from the last physical/remote device.
            let baud = self
                .pairs
                .iter()
                .find(|pair| pair.paths.contains(&path))
                .and_then(|pair| match pair.timing {
                    signal_forge::virtual_pair::LinkTiming::Emulated(frame) => Some(frame.baud),
                    _ => None,
                })
                .unwrap_or(19200);
            self.settings = SerialSettings {
                path,
                baud,
                ..Default::default()
            };
            self.baud_control = BaudControl::new(baud);
            self.connect();
        }
        if let Some(index) = remove {
            let pair = &self.pairs[index];
            for (_, tab) in self.dock.iter_all_tabs_mut() {
                if pair
                    .paths
                    .iter()
                    .chain(&pair.raw_paths)
                    .any(|path| path == tab.endpoint.display_name())
                {
                    tab.endpoint.disconnect();
                }
            }
            self.pairs.remove(index);
        }
        ui.separator();
        ui.heading("Bridges");
        if ui.small_button("+ Bridge").clicked() {
            self.open_setup(workbench_ui::SetupKind::Bridge);
        }
        for (index, view) in self.bridges.iter().enumerate() {
            if ui
                .selectable_label(
                    self.active_bridge == index,
                    format!(
                        "{} <-> {}",
                        view.bridge.a.0.trim_start_matches("serial:"),
                        view.bridge.b.0.trim_start_matches("serial:")
                    ),
                )
                .clicked()
            {
                self.active_bridge = index;
            }
            ui.label(
                RichText::new(format!("{:?}", view.bridge.state()))
                    .small()
                    .color(theme::MUTED),
            );
        }
        if self.bridges.is_empty() {
            ui.weak("No bridges running");
        }
    }

    pub(super) fn bridge_monitors(&mut self, ctx: &egui::Context) {
        if self.bridges.is_empty() {
            return;
        }
        // Drain every bridge even when another inspector tab is selected.
        for view in &mut self.bridges {
            for event in view.events.try_iter().take(2048) {
                view.inspector.receive(event, &view.bridge.a);
            }
            if view.bridge.state() != signal_forge::bridge::BridgeState::Running {
                if let Some(capture) = &view.capture {
                    capture.request_stop();
                }
            }
        }
        self.active_bridge = self.active_bridge.min(self.bridges.len() - 1);
        let mut remove = false;
        egui::TopBottomPanel::bottom("bridge-monitors")
            .resizable(true)
            .min_height(120.0)
            .default_height(220.0)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong("Traffic inspector");
                    egui::ComboBox::from_id_salt("inspector-bridge")
                        .selected_text(format!("Bridge {}", self.active_bridge + 1))
                        .show_ui(ui, |ui| {
                            for (index, view) in self.bridges.iter().enumerate() {
                                ui.selectable_value(
                                    &mut self.active_bridge,
                                    index,
                                    format!(
                                        "{} <-> {}",
                                        view.bridge.a.0.trim_start_matches("serial:"),
                                        view.bridge.b.0.trim_start_matches("serial:")
                                    ),
                                );
                            }
                        });
                    let view = &self.bridges[self.active_bridge];
                    ui.label(format!(
                        "{} <-> {}",
                        view.bridge.a.0.trim_start_matches("serial:"),
                        view.bridge.b.0.trim_start_matches("serial:")
                    ));
                    ui.colored_label(
                        if view.bridge.state() == signal_forge::bridge::BridgeState::Running {
                            theme::CONNECTED
                        } else {
                            theme::ERROR
                        },
                        format!("{:?}", view.bridge.state()),
                    );
                    if ui.add(theme::danger_button("Stop / remove")).clicked() {
                        remove = true;
                    }
                });
                let view = &mut self.bridges[self.active_bridge];
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(&mut view.inspector.filter, DirectionFilter::Both, "Both");
                    ui.selectable_value(&mut view.inspector.filter, DirectionFilter::AToB, "A > B");
                    ui.selectable_value(&mut view.inspector.filter, DirectionFilter::BToA, "B > A");
                    ui.checkbox(&mut view.inspector.deltas, "Delta");
                    ui.checkbox(&mut view.inspector.paused, "Pause")
                        .on_hover_text("Forwarding and capture continue");
                    ui.checkbox(&mut view.inspector.auto_scroll, "Autoscroll");
                    if ui.button("Clear").clicked() {
                        view.inspector.clear();
                    }
                    let recording = view.capture.as_ref().is_some_and(|c| {
                        matches!(
                            c.status().state,
                            CaptureState::Recording | CaptureState::Finishing
                        )
                    });
                    ui.menu_button("Capture file…", |ui| {
                        ui.add_enabled(
                            !recording,
                            egui::TextEdit::singleline(&mut view.capture_path).desired_width(280.0),
                        );
                        ui.weak("JSON Lines · both directions · independent of display");
                    });
                    let finishing = view
                        .capture
                        .as_ref()
                        .is_some_and(|c| c.status().state == CaptureState::Finishing);
                    if ui
                        .add_enabled(
                            !finishing,
                            theme::primary_button(if recording {
                                "Stop capture"
                            } else {
                                "Record"
                            }),
                        )
                        .clicked()
                    {
                        view.toggle_capture();
                    }
                    if let Some(capture) = &view.capture {
                        let status = capture.status();
                        ui.label(
                            RichText::new(format!(
                                "{:?} · {} B · {} dropped",
                                status.state, status.bytes, status.dropped
                            ))
                            .small()
                            .color(if status.dropped > 0 {
                                theme::WARNING
                            } else {
                                theme::MUTED
                            }),
                        );
                    }
                });
                if let Some(error) = &view.capture_error {
                    ui.colored_label(theme::ERROR, error);
                }
                if view.bridge.dropped_events() > 0 {
                    ui.colored_label(
                        theme::WARNING,
                        format!(
                            "{} monitor chunks dropped; forwarding continues",
                            view.bridge.dropped_events()
                        ),
                    );
                }
                let delta_width = if view.inspector.deltas { 70.0 } else { 0.0 };
                let payload_width = ((ui.available_width() - 210.0 - delta_width) / 2.0).max(100.0);
                ui.horizontal(|ui| {
                    for (label, width) in
                        [("Time (UTC)", 85.0), ("Direction", 52.0), ("Bytes", 40.0)]
                    {
                        ui.add_sized(
                            [width, 18.0],
                            egui::Label::new(RichText::new(label).color(theme::MUTED)),
                        );
                    }
                    if view.inspector.deltas {
                        ui.add_sized([delta_width, 18.0], egui::Label::new("Delta (ms)"));
                    }
                    for label in ["ASCII", "Hex"] {
                        ui.add_sized(
                            [payload_width, 18.0],
                            egui::Label::new(RichText::new(label).color(theme::MUTED)),
                        );
                    }
                });
                ui.separator();
                let rows: Vec<_> = view
                    .inspector
                    .rows
                    .iter()
                    .filter(|row| view.inspector.filter.accepts(row.direction))
                    .collect();
                egui::ScrollArea::both()
                    .id_salt(("bridge-traffic", self.active_bridge))
                    .auto_shrink([false, false])
                    .stick_to_bottom(view.inspector.auto_scroll)
                    .max_height(ui.available_height())
                    .show_rows(ui, theme::TRAFFIC_ROW_HEIGHT, rows.len(), |ui, range| {
                        for index in range {
                            let row = rows[index];
                            let time = timestamp_utc(row.event.timestamp);
                            let direction_color = if row.direction
                                == signal_forge::inspector::BridgeDirection::AToB
                            {
                                theme::TX
                            } else {
                                theme::RX
                            };
                            ui.horizontal(|ui| {
                                ui.add_sized(
                                    [85.0, 18.0],
                                    egui::Label::new(
                                        RichText::new(&time[11..23])
                                            .monospace()
                                            .color(theme::MUTED),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(format!(
                                    "{} · chunk #{} · {} missed chunks",
                                    time, row.event.sequence, row.missed_before
                                ));
                                ui.add_sized(
                                    [52.0, 18.0],
                                    egui::Label::new(
                                        RichText::new(
                                            if row.direction
                                                == signal_forge::inspector::BridgeDirection::AToB
                                            {
                                                "A > B"
                                            } else {
                                                "B > A"
                                            },
                                        )
                                        .color(direction_color),
                                    ),
                                );
                                ui.add_sized(
                                    [40.0, 18.0],
                                    egui::Label::new(row.event.bytes.len().to_string()),
                                );
                                if view.inspector.deltas {
                                    ui.add_sized(
                                        [delta_width, 18.0],
                                        egui::Label::new(
                                            row.delta_ns
                                                .map(|ns| {
                                                    format!("{:+.3}", ns as f64 / 1_000_000.0)
                                                })
                                                .unwrap_or_else(|| "—".into()),
                                        ),
                                    );
                                }
                                for text in [
                                    traffic::ascii(&row.event.bytes),
                                    traffic::hex(&row.event.bytes),
                                ] {
                                    ui.add_sized(
                                        [payload_width, 18.0],
                                        egui::Label::new(RichText::new(&text).monospace())
                                            .truncate(),
                                    )
                                    .on_hover_text(text);
                                }
                                if row.missed_before > 0 {
                                    ui.colored_label(theme::WARNING, "GAP");
                                }
                            });
                        }
                    });
            });
        if remove {
            self.bridges.remove(self.active_bridge);
        }
    }
}

impl Drop for Workbench {
    fn drop(&mut self) {
        self.finish_session_outputs();
        self.save_workspace();
        self.snapshot_workspace();
        if let Err(error) = self.save_session_context(true) {
            log::error!("Session finalization: {error}");
        }
        self.bridges.clear();
        for (_, tab) in self.dock.iter_all_tabs_mut() {
            tab.endpoint.disconnect();
        }
        self.pairs.clear();
    }
}

impl BridgeView {
    pub(super) fn set_capture_path(&mut self, path: std::path::PathBuf) {
        self.capture_path = path.display().to_string();
    }
    pub(super) fn clear_session_path(&mut self, root: &Path) {
        if Path::new(&self.capture_path).starts_with(root) {
            self.capture_path = format!(
                "bridge-{}.jsonl",
                signal_forge::inspector::timestamp_ns(std::time::SystemTime::now())
            );
        }
    }
    pub(super) fn finish_capture(&mut self) {
        if let Some(capture) = &mut self.capture {
            capture.finish();
        }
    }

    fn toggle_capture(&mut self) {
        if let Some(capture) = &self.capture {
            if matches!(
                capture.status().state,
                CaptureState::Recording | CaptureState::Finishing
            ) {
                capture.request_stop();
                return;
            }
        }
        match Capture::start(Path::new(&self.capture_path), &self.bridge) {
            Ok(capture) => {
                log::info!("Capture started: {}", capture.path.display());
                self.pending_artifact = Some(capture.path.clone());
                self.capture = Some(capture);
                self.capture_error = None;
            }
            Err(error) => self.capture_error = Some(error),
        }
    }
}
impl Drop for BridgeView {
    fn drop(&mut self) {
        self.bridge.stop();
        if let Some(capture) = &mut self.capture {
            capture.finish();
        }
    }
}
