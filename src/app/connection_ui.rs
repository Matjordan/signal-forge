use super::*;
use signal_forge::{bridge::Bridge, virtual_pair::VirtualPair};
use signal_forge::{
    capture::{Capture, CaptureState},
    inspector::{timestamp_utc, DirectionFilter, Inspector},
};
use std::path::Path;

pub(super) struct BridgeView {
    bridge: Bridge,
    events: Receiver<Arc<TrafficEvent>>,
    inspector: Inspector,
    capture_path: String,
    capture: Option<Capture>,
    capture_error: Option<String>,
}
impl Workbench {
    fn create_pair(&mut self) {
        if self.pairs.iter().any(|p| p.name == self.pair_name) {
            self.error = Some(format!("PTY pair {} already exists", self.pair_name));
        } else {
            let directory = if self.pair_directory.trim().is_empty() {
                None
            } else {
                Some(Path::new(&self.pair_directory))
            };
            match VirtualPair::create(&self.pair_name, directory) {
                Ok(pair) => {
                    log::info!(
                        "Created PTY pair {}: {} <-> {}",
                        pair.name,
                        pair.paths[0],
                        pair.paths[1]
                    );
                    self.pairs.push(pair);
                    self.error = None;
                }
                Err(error) => self.error = Some(format!("PTY pair {}: {error}", self.pair_name)),
            }
        }
    }
    fn start_bridge(&mut self) {
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
                    events,
                    inspector: Inspector::new(),
                    capture_path: format!(
                        "bridge-{}.jsonl",
                        signal_forge::inspector::timestamp_ns(std::time::SystemTime::now())
                    ),
                    capture: None,
                    capture_error: None,
                });
                self.error = None;
            }
            Err(error) => {
                self.error = Some(format!(
                    "Bridge {} ↔ {}: {error}",
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
        if ctx.wants_keyboard_input() {
            return;
        }
        let modifiers = egui::Modifiers::CTRL | egui::Modifiers::SHIFT;
        if ctx.input_mut(|i| i.consume_key(modifiers, egui::Key::N)) {
            self.create_pair();
        }
        if ctx.input_mut(|i| i.consume_key(modifiers, egui::Key::B)) {
            self.start_bridge();
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
        ui.text_edit_singleline(&mut self.pair_name);
        ui.label("Link directory (optional)");
        ui.text_edit_singleline(&mut self.pair_directory);
        if ui.button("Create PTY pair").clicked() {
            self.create_pair();
        }
        ui.small("Ctrl+Shift+N creates a pair");
        let mut open = None;
        let mut remove = None;
        for (index, pair) in self.pairs.iter().enumerate() {
            ui.push_id(index, |ui| {
                ui.label(format!("{} · {:?}", pair.name, pair.state()));
                for (side, path) in pair.paths.iter().enumerate() {
                    ui.small(format!("{}: {path}", if side == 0 { "A" } else { "B" }));
                    ui.horizontal(|ui| {
                        if ui
                            .small_button(format!("Open {}", if side == 0 { "A" } else { "B" }))
                            .clicked()
                        {
                            open = Some(path.clone());
                        }
                        if ui.small_button("Copy path").clicked() {
                            ui.ctx().copy_text(path.clone());
                        }
                    });
                }
                if ui.small_button("Remove pair").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(path) = open {
            self.settings.path = path;
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
        let endpoints: Vec<_> = self
            .dock
            .iter_all_tabs()
            .filter(|(_, t)| t.endpoint.state() == ConnectionState::Connected)
            .map(|(_, t)| {
                (
                    t.endpoint.id().clone(),
                    t.endpoint.display_name().to_owned(),
                )
            })
            .collect();
        if self.bridge_a.is_none() {
            self.bridge_a = endpoints.first().map(|e| e.0.clone());
        }
        if self.bridge_b.is_none() {
            self.bridge_b = endpoints.get(1).map(|e| e.0.clone());
        }
        for (label, selection) in [("A", &mut self.bridge_a), ("B", &mut self.bridge_b)] {
            egui::ComboBox::from_id_salt(format!("bridge-{label}"))
                .selected_text(format!(
                    "{label}: {}",
                    selection
                        .as_ref()
                        .map(|id| id.0.as_str())
                        .unwrap_or("Select endpoint")
                ))
                .show_ui(ui, |ui| {
                    for (id, name) in &endpoints {
                        ui.selectable_value(selection, Some(id.clone()), name);
                    }
                });
        }
        if ui.button("Start full-duplex bridge").clicked() {
            self.start_bridge();
        }
        ui.small("Ctrl+Shift+B starts the selected bridge");
        ui.small("RX on A → TX on B; RX on B → TX on A.");
    }
    pub(super) fn bridge_monitors(&mut self, ctx: &egui::Context) {
        if self.bridges.is_empty() {
            return;
        }
        let mut remove = None;
        egui::TopBottomPanel::bottom("bridge-monitors").resizable(true).min_height(170.0).default_height(280.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (index, view) in self.bridges.iter_mut().enumerate() {
                    for event in view.events.try_iter().take(2048) { view.inspector.receive(event, &view.bridge.a); }
                    if view.bridge.state() != signal_forge::bridge::BridgeState::Running {
                        if let Some(capture) = &view.capture { capture.request_stop(); }
                    }
                    ui.push_id(index, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(format!("Bridge {} ↔ {}", view.bridge.a.0, view.bridge.b.0));
                            ui.label(format!("{:?}", view.bridge.state()));
                            if ui.button("Stop / remove").clicked() { remove = Some(index); }
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.selectable_value(&mut view.inspector.filter, DirectionFilter::Both, "Both directions");
                            ui.selectable_value(&mut view.inspector.filter, DirectionFilter::AToB, "A → B");
                            ui.selectable_value(&mut view.inspector.filter, DirectionFilter::BToA, "B → A");
                            ui.checkbox(&mut view.inspector.deltas, "Delta times");
                            ui.checkbox(&mut view.inspector.paused, "Pause display").on_hover_text("Capture and forwarding continue. Ctrl+Shift+M toggles all bridge displays.");
                            ui.checkbox(&mut view.inspector.auto_scroll, "Auto-scroll");
                            if ui.button("Clear display").clicked() { view.inspector.clear(); }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Capture file");
                            let recording = view.capture.as_ref().is_some_and(|c| matches!(c.status().state, CaptureState::Recording | CaptureState::Finishing));
                            ui.add_enabled(!recording, egui::TextEdit::singleline(&mut view.capture_path).desired_width(240.0));
                            let finishing = view.capture.as_ref().is_some_and(|c| c.status().state == CaptureState::Finishing);
                            if ui.add_enabled(!finishing, egui::Button::new(if recording { "Stop capture" } else { "Start capture" })).clicked() { view.toggle_capture(); }
                        });
                        if let Some(capture) = &view.capture {
                            let status = capture.status();
                            ui.small(format!("Capture {:?} · {} chunks · {} bytes · {} dropped", status.state, status.events, status.bytes, status.dropped));
                            if status.dropped > 0 { ui.colored_label(Color32::YELLOW, "Capture is incomplete: its queue dropped events."); }
                        } else { ui.small("JSON Lines · both directions · independent of filters/pause · Ctrl+Shift+R toggles first bridge capture"); }
                        if let Some(error) = &view.capture_error { ui.colored_label(Color32::LIGHT_RED, error); }
                        let dropped = view.bridge.dropped_events();
                        if dropped > 0 { ui.colored_label(Color32::YELLOW,format!("{dropped} monitor events dropped; forwarding remains independent")); }
                        let rows: Vec<_> = view.inspector.rows.iter().filter(|row| view.inspector.filter.accepts(row.direction)).collect();
                        let row_height = ui.text_style_height(&egui::TextStyle::Monospace) * 3.0 + ui.spacing().item_spacing.y * 2.0;
                        egui::ScrollArea::both().id_salt("bridge-traffic").max_height(150.0).auto_shrink([false,false]).stick_to_bottom(view.inspector.auto_scroll).show_rows(ui, row_height, rows.len(), |ui, range| {
                            for index in range {
                                let row = rows[index];
                                let event = &row.event;
                                let delta = if view.inspector.deltas { row.delta_ns.map(|ns| format!("  Δ {:+.3} ms", ns as f64 / 1_000_000.0)).unwrap_or_else(|| "  Δ —".into()) } else { String::new() };
                                let gap = if row.missed_before > 0 { format!("  GAP: {} chunks", row.missed_before) } else { String::new() };
                                for line in [format!("#{:06}  {}  {}{delta}{gap}", event.sequence, timestamp_utc(event.timestamp), row.direction.label()), format!("ASCII  {}", traffic::ascii(&event.bytes)), format!("HEX    {}", traffic::hex(&event.bytes))] {
                                    ui.add(egui::Label::new(RichText::new(line).monospace()).wrap_mode(egui::TextWrapMode::Extend));
                                }
                            }
                        });
                    });
                }
            });
        });
        if let Some(index) = remove {
            self.bridges.remove(index);
        }
    }
}
impl Drop for Workbench {
    fn drop(&mut self) {
        self.save_workspace();
        self.bridges.clear();
        for (_, tab) in self.dock.iter_all_tabs_mut() {
            tab.endpoint.disconnect();
        }
        self.pairs.clear();
    }
}

impl BridgeView {
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
