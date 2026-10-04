use super::*;
use signal_forge::{bridge::Bridge, virtual_pair::VirtualPair};
use std::{collections::VecDeque, path::Path};

pub(super) struct BridgeView {
    bridge: Bridge,
    events: Receiver<Arc<TrafficEvent>>,
    history: VecDeque<Arc<TrafficEvent>>,
    paused: bool,
}
impl Workbench {
    pub(super) fn connections_ui(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.heading("Virtual pairs");
        ui.text_edit_singleline(&mut self.pair_name);
        ui.label("Link directory (optional)");
        ui.text_edit_singleline(&mut self.pair_directory);
        if ui.button("Create PTY pair").clicked() {
            if self.pairs.iter().any(|p| p.name == self.pair_name) {
                self.error = Some("A pair with this name already exists".into());
            } else {
                let directory = if self.pair_directory.trim().is_empty() { None } else { Some(Path::new(&self.pair_directory)) };
                match VirtualPair::create(&self.pair_name, directory) {
                    Ok(pair) => { self.pairs.push(pair); self.error = None; }
                    Err(error) => self.error = Some(error),
                }
            }
        }
        let mut open = None;
        let mut remove = None;
        for (index, pair) in self.pairs.iter().enumerate() {
            ui.push_id(index, |ui| {
                ui.label(format!("{} · {:?}", pair.name, pair.state()));
                for (side,path) in pair.paths.iter().enumerate() {
                    ui.small(format!("{}: {path}", if side == 0 { "A" } else { "B" }));
                    ui.horizontal(|ui| {
                        if ui.small_button(format!("Open {}", if side == 0 { "A" } else { "B" })).clicked() { open = Some(path.clone()); }
                        if ui.small_button("Copy path").clicked() { ui.ctx().copy_text(path.clone()); }
                    });
                }
                if ui.small_button("Remove pair").clicked() { remove = Some(index); }
            });
        }
        if let Some(path) = open { self.settings.path = path; self.connect(); }
        if let Some(index) = remove {
            let pair = &self.pairs[index];
            for (_,tab) in self.dock.iter_all_tabs_mut() {
                if pair.paths.iter().chain(&pair.raw_paths).any(|path| path == tab.endpoint.display_name()) { tab.endpoint.disconnect(); }
            }
            self.pairs.remove(index);
        }
        ui.separator();
        ui.heading("Bridges");
        let endpoints: Vec<_> = self.dock.iter_all_tabs().filter(|(_,t)| t.endpoint.state() == ConnectionState::Connected).map(|(_,t)| (t.endpoint.id().clone(), t.endpoint.display_name().to_owned())).collect();
        if self.bridge_a.is_none() { self.bridge_a = endpoints.first().map(|e| e.0.clone()); }
        if self.bridge_b.is_none() { self.bridge_b = endpoints.get(1).map(|e| e.0.clone()); }
        for (label, selection) in [("A", &mut self.bridge_a), ("B", &mut self.bridge_b)] {
            egui::ComboBox::from_id_salt(format!("bridge-{label}"))
                .selected_text(format!("{label}: {}", selection.as_ref().map(|id| id.0.as_str()).unwrap_or("Select endpoint")))
                .show_ui(ui, |ui| {
                    for (id,name) in &endpoints { ui.selectable_value(selection, Some(id.clone()), name); }
                });
        }
        if ui.button("Start full-duplex bridge").clicked() {
            let ports: Result<Vec<_>, _> = [&self.bridge_a, &self.bridge_b].iter().map(|selection| {
                self.dock.iter_all_tabs().find(|(_,t)| Some(t.endpoint.id()) == selection.as_ref()).ok_or(EndpointError::Disconnected)?.1.endpoint.bridge_port()
            }).collect();
            match ports.and_then(|ports| Bridge::start(ports[0].clone(), ports[1].clone())) {
                Ok(bridge) => { let events = bridge.subscribe(512); self.bridges.push(BridgeView { bridge,events,history:VecDeque::new(),paused:false }); self.error = None; }
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        ui.small("RX on A → TX on B; RX on B → TX on A.");
    }
    pub(super) fn bridge_monitors(&mut self, ctx: &egui::Context) {
        if self.bridges.is_empty() { return; }
        let mut remove = None;
        egui::TopBottomPanel::bottom("bridge-monitors").resizable(true).default_height(170.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (index, view) in self.bridges.iter_mut().enumerate() {
                    for event in view.events.try_iter().take(2048) {
                        if !view.paused { view.history.push_back(event); }
                    }
                    while view.history.len() > 256 { view.history.pop_front(); }
                    ui.push_id(index, |ui| {
                        ui.horizontal(|ui| {
                            ui.strong(format!("Bridge {} ↔ {}", view.bridge.a.0, view.bridge.b.0));
                            ui.label(format!("{:?}", view.bridge.state()));
                            ui.checkbox(&mut view.paused,"Pause display");
                            if ui.button("Stop / remove").clicked() { remove = Some(index); }
                        });
                        let dropped = view.bridge.dropped_events();
                        if dropped > 0 { ui.colored_label(Color32::YELLOW,format!("{dropped} monitor events dropped; forwarding remains independent")); }
                        egui::ScrollArea::vertical().id_salt("bridge-traffic").max_height(95.0).stick_to_bottom(true).show(ui, |ui| {
                            for event in &view.history {
                                let direction = if event.endpoint == view.bridge.a { "A → B" } else { "B → A" };
                                ui.monospace(format!("#{:06}  {direction}  {}", event.sequence, traffic::ascii(&event.bytes)));
                            }
                        });
                    });
                }
            });
        });
        if let Some(index) = remove { self.bridges.remove(index); }
    }
}
impl Drop for Workbench {
    fn drop(&mut self) {
        self.bridges.clear();
        for (_,tab) in self.dock.iter_all_tabs_mut() { tab.endpoint.disconnect(); }
        self.pairs.clear();
    }
}
