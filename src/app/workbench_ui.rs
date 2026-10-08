use super::*;
use egui_dock::TabIndex;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SetupKind {
    Port,
    Pair,
    Bridge,
    Workspace,
}

pub(super) struct SetupDialog {
    kind: SetupKind,
    settings: SerialSettings,
    baud: BaudControl,
    pair_name: String,
    directory: String,
    a: Option<EndpointId>,
    b: Option<EndpointId>,
    workspace_path: String,
    remote: bool,
    host_name: String,
    host: signal_forge::ssh_serial::SshHost,
    device: String,
    discovery: Option<signal_forge::ssh_serial::Discovery>,
    devices: Vec<String>,
    error: Option<String>,
}

pub(super) fn framing(settings: &SerialSettings) -> String {
    format!(
        "{} · {}{}{} · {:?}",
        settings.baud,
        settings.data_bits,
        match settings.parity {
            Parity::None => "N",
            Parity::Odd => "O",
            Parity::Even => "E",
        },
        settings.stop_bits,
        settings.flow
    )
}

impl Workbench {
    pub(super) fn open_setup(&mut self, kind: SetupKind) {
        let connected: Vec<_> = self
            .dock
            .iter_all_tabs()
            .filter(|(_, tab)| tab.endpoint.state() == ConnectionState::Connected)
            .map(|(_, tab)| tab.endpoint.id().clone())
            .collect();
        let parsed = signal_forge::ssh_serial::SshHost::parse(&self.settings.path).ok();
        self.setup = Some(SetupDialog {
            remote: parsed.is_some(),
            host_name: parsed
                .as_ref()
                .map(|p| p.0.host.clone())
                .unwrap_or_default(),
            host: parsed.as_ref().map(|p| p.0.clone()).unwrap_or(
                signal_forge::ssh_serial::SshHost {
                    host: String::new(),
                    username: None,
                    port: None,
                },
            ),
            device: parsed
                .map(|p| p.1)
                .unwrap_or_else(|| self.settings.path.clone()),
            discovery: None,
            devices: Vec::new(),
            kind,
            settings: self.settings.clone(),
            baud: BaudControl::new(self.settings.baud),
            pair_name: self.pair_name.clone(),
            directory: self.pair_directory.clone(),
            a: self.bridge_a.clone().or_else(|| connected.first().cloned()),
            b: self.bridge_b.clone().or_else(|| connected.get(1).cloned()),
            workspace_path: WorkspaceConfig::path()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            error: None,
        });
    }
    pub(super) fn toolbar_ui(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("Signal Forge")
                        .strong()
                        .size(22.0)
                        .color(theme::TEXT),
                );
                ui.add_space(12.0);
                for (label, kind) in [
                    ("+ New Port", SetupKind::Port),
                    ("Virtual Pair", SetupKind::Pair),
                    ("Bridge", SetupKind::Bridge),
                ] {
                    if ui.button(label).clicked() {
                        self.open_setup(kind);
                    }
                }
                if ui
                    .add_enabled(self.config_recoverable, egui::Button::new("Save Workspace"))
                    .clicked()
                {
                    self.save_workspace();
                }
                if ui.button("Workspace…").clicked() {
                    self.open_setup(SetupKind::Workspace);
                }
                if ui
                    .button("2 Tiles")
                    .on_hover_text("Arrange open terminals in two columns")
                    .clicked()
                {
                    self.arrange_tiles(false);
                }
                if ui
                    .button("4 Tiles")
                    .on_hover_text("Arrange open terminals in a 2 × 2 grid")
                    .clicked()
                {
                    self.arrange_tiles(true);
                }
                if ui.button("Presets…").clicked() {
                    self.preset_library_open = true;
                }
                if ui.button("Refresh").clicked() {
                    self.refresh();
                }
            });
        });
    }
    pub(super) fn arrange_tiles(&mut self, grid: bool) {
        let mut old = std::mem::replace(&mut self.dock, DockState::new(Vec::new()));
        let mut tabs = Vec::new();
        loop {
            let location = old
                .iter_all_tabs()
                .next()
                .map(|((surface, node), _)| (surface, node, TabIndex(0)));
            let Some(location) = location else {
                break;
            };
            if let Some(tab) = old.remove_tab(location) {
                tabs.push(tab);
            } else {
                break;
            }
        }
        let mut tabs = tabs.into_iter();
        let Some(first) = tabs.next() else {
            return;
        };
        self.dock = DockState::new(vec![first]);
        if let Some(second) = tabs.next() {
            let [left, right] =
                self.dock
                    .main_surface_mut()
                    .split_right(NodeIndex::root(), 0.5, vec![second]);
            if grid {
                if let Some(third) = tabs.next() {
                    self.dock
                        .main_surface_mut()
                        .split_below(left, 0.5, vec![third]);
                }
                if let Some(fourth) = tabs.next() {
                    self.dock
                        .main_surface_mut()
                        .split_below(right, 0.5, vec![fourth]);
                }
            }
        }
        for tab in tabs {
            self.dock.push_to_focused_leaf(tab);
        }
    }
    fn select_terminal(&mut self, id: EndpointId) {
        let location = self
            .dock
            .iter_all_tabs()
            .find(|(_, t)| *t.endpoint.id() == id)
            .map(|((surface, node), _)| (surface, node));
        if let Some((surface, node)) = location {
            if let egui_dock::Node::Leaf { tabs, active, .. } = &mut self.dock[surface][node] {
                if let Some(index) = tabs.iter().position(|tab| *tab.endpoint.id() == id) {
                    *active = TabIndex(index);
                }
            }
            self.dock.set_focused_node_and_surface((surface, node));
        }
        self.selected = Some(id);
    }
    pub(super) fn workbench_sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("devices")
            .resizable(true)
            .default_width(235.0)
            .width_range(190.0..=320.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("connections-sidebar")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.heading("Endpoints");
                            if ui.small_button("+").on_hover_text("Open port").clicked() {
                                self.open_setup(SetupKind::Port);
                            }
                        });
                        let entries: Vec<_> = self
                            .dock
                            .iter_all_tabs()
                            .map(|(_, tab)| {
                                (
                                    tab.endpoint.id().clone(),
                                    tab.settings.path.clone(),
                                    tab.endpoint.state(),
                                    framing(&tab.settings),
                                )
                            })
                            .collect();
                        for (id, path, state, summary) in &entries {
                            let color = match state {
                                ConnectionState::Connected => theme::CONNECTED,
                                ConnectionState::Connecting => theme::MUTED,
                                ConnectionState::Disconnected => theme::MUTED,
                                ConnectionState::Fault(_) => theme::ERROR,
                            };
                            theme::card_frame().show(ui, |ui| {
                                if ui
                                    .selectable_label(
                                        self.selected.as_ref() == Some(id),
                                        RichText::new(format!("{path}")).color(color),
                                    )
                                    .clicked()
                                {
                                    self.select_terminal(id.clone());
                                }
                                ui.label(RichText::new(summary).small().color(theme::MUTED));
                            });
                        }
                        if entries.is_empty() {
                            ui.weak("No open terminals");
                        }
                        egui::CollapsingHeader::new("Available / recent devices")
                            .default_open(entries.is_empty())
                            .show(ui, |ui| {
                                let mut paths = self.ports.clone();
                                for saved in &self.config.ports {
                                    if !paths.contains(&saved.path) {
                                        paths.push(saved.path.clone());
                                    }
                                }
                                egui::ScrollArea::vertical()
                                    .max_height(160.0)
                                    .show(ui, |ui| {
                                        for path in paths {
                                            if ui.button(&path).clicked() {
                                                self.open_setup(SetupKind::Port);
                                                if let Some(dialog) = &mut self.setup {
                                                    dialog.settings = self
                                                        .config
                                                        .ports
                                                        .iter()
                                                        .find(|s| s.path == path)
                                                        .cloned()
                                                        .unwrap_or_else(|| SerialSettings {
                                                            path,
                                                            ..Default::default()
                                                        });
                                                    dialog.baud =
                                                        BaudControl::new(dialog.settings.baud);
                                                    dialog.remote =
                                                        dialog.settings.path.starts_with("ssh://");
                                                    if let Ok((host, device)) =
                                                        signal_forge::ssh_serial::SshHost::parse(
                                                            &dialog.settings.path,
                                                        )
                                                    {
                                                        dialog.host_name = host.host.clone();
                                                        dialog.host = host;
                                                        dialog.device = device;
                                                    } else {
                                                        dialog.device =
                                                            dialog.settings.path.clone();
                                                    }
                                                }
                                            }
                                        }
                                    });
                            });
                        self.connections_ui(ui);
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.heading("Presets");
                            if ui.small_button("Manage…").clicked() {
                                self.preset_library_open = true;
                            }
                        });
                        self.quick_presets(ui);
                    });
            });
    }
    pub(super) fn quick_presets(&mut self, ui: &mut egui::Ui) {
        egui::ComboBox::from_id_salt("quick-profile")
            .selected_text(&self.library.profiles[self.profile_index].name)
            .show_ui(ui, |ui| {
                for (index, profile) in self.library.profiles.iter().enumerate() {
                    ui.selectable_value(&mut self.profile_index, index, &profile.name);
                }
            });
        ui.label(
            RichText::new(
                self.selected
                    .as_ref()
                    .map(|id| format!("Target: {}", id.0.trim_start_matches("serial:")))
                    .unwrap_or_else(|| "Select a terminal to send".into()),
            )
            .small()
            .color(theme::MUTED),
        );
        let presets = self.library.profiles[self.profile_index].presets.clone();
        if presets.is_empty() {
            ui.weak("No presets yet. Use Manage to add one.");
        }
        for preset in presets {
            if ui
                .add_sized(
                    [ui.available_width(), 24.0],
                    egui::Button::new(&preset.name),
                )
                .on_hover_text(format!("{}\n{}", preset.description, preset.shortcut))
                .clicked()
            {
                self.dispatch_preset(preset);
            }
        }
    }
    pub(super) fn preset_library_ui(&mut self, ctx: &egui::Context) {
        let mut open = self.preset_library_open;
        egui::Window::new("Preset library")
            .open(&mut open)
            .default_width(380.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(600.0)
                    .show(ui, |ui| self.presets_ui(ui));
            });
        self.preset_library_open = open;
    }
    pub(super) fn setup_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.setup.take() else {
            return;
        };
        let mut action = 0;
        let response = egui::Modal::new(egui::Id::new("setup-dialog")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading(match dialog.kind { SetupKind::Port => "Open serial port", SetupKind::Pair => "Create virtual pair", SetupKind::Bridge => "Create full-duplex bridge", SetupKind::Workspace => "Workspace" });
            ui.separator();
            match dialog.kind {
                SetupKind::Port => {
                    ui.checkbox(&mut dialog.remote, "Remote serial over SSH");
                    if dialog.remote {
                        egui::ComboBox::from_id_salt("saved-ssh-hosts").selected_text("Saved SSH hosts").show_ui(ui, |ui| {
                            for saved in &self.config.remote_hosts {
                                if ui.selectable_label(dialog.host_name == saved.name, &saved.name).clicked() {
                                    dialog.host = saved.connection.clone(); dialog.host_name = saved.name.clone();
                                }
                            }
                        });
                        ui.label("Display name"); ui.text_edit_singleline(&mut dialog.host_name);
                        ui.label("SSH host or config alias"); ui.text_edit_singleline(&mut dialog.host.host);
                        let mut user = dialog.host.username.clone().unwrap_or_default();
                        ui.label("Username (blank uses SSH config)"); ui.text_edit_singleline(&mut user);
                        dialog.host.username = (!user.is_empty()).then_some(user);
                        let mut port = dialog.host.port.unwrap_or(0);
                        ui.horizontal(|ui| {ui.label("SSH port (0 uses config)"); ui.add(egui::DragValue::new(&mut port));});
                        dialog.host.port = (port != 0).then_some(port);
                        ui.horizontal(|ui| {
                            if ui.button("Save host").clicked() {
                                if let Err(error) = dialog.host.validate() {dialog.error = Some(error.to_string());}
                                else if dialog.host_name.is_empty() || dialog.host_name.len() > 256 || dialog.host_name.contains(['\0', '\n', '\r']) {dialog.error = Some("Enter a valid display name".into());}
                                else if self.config.remote_hosts.len() >= 128 && !self.config.remote_hosts.iter().any(|host| host.name == dialog.host_name) {dialog.error = Some("At most 128 SSH hosts can be saved".into());}
                                else {
                                    self.config.remote_hosts.retain(|host| host.name != dialog.host_name);
                                    self.config.remote_hosts.push(signal_forge::ssh_serial::SavedSshHost {name: dialog.host_name.clone(), connection: dialog.host.clone()});
                                    self.save_workspace(); dialog.error = self.error.clone();
                                }
                            }
                            if ui.button("Remove saved host").clicked() {
                                self.config.remote_hosts.retain(|host| host.name != dialog.host_name); self.save_workspace();
                            }
                        });
                        ui.weak("Uses SSH keys/agent and strict known-host verification. Remote host needs Python 3.");
                        if ui.add_enabled(dialog.discovery.is_none(), egui::Button::new("Discover remote devices")).clicked() {
                            dialog.discovery = Some(signal_forge::ssh_serial::Discovery::start(dialog.host.clone()));
                        }
                        if let Some(rx) = &dialog.discovery {
                            if let Some(result) = rx.try_result() {
                                dialog.discovery = None;
                                match result {Ok(paths) => {dialog.devices = paths; dialog.error = None;}, Err(e) => dialog.error = Some(e)}
                            } else {ui.label("Connecting / discovering…"); ctx.request_repaint_after(Duration::from_millis(100));}
                        }
                        egui::ComboBox::from_id_salt("remote-devices").selected_text("Discovered devices").show_ui(ui, |ui| {
                            for path in &dialog.devices {ui.selectable_value(&mut dialog.device, path.clone(), path);}
                        });
                        dialog.settings.path = dialog.device.clone();
                        serial_form(ui, &mut dialog.settings, &mut dialog.baud);
                        dialog.device = dialog.settings.path.clone();
                        dialog.settings.path = dialog.host.uri(&dialog.device);
                    } else {
                        if dialog.settings.path.starts_with("ssh://") {dialog.settings.path = String::new();}
                        serial_form(ui, &mut dialog.settings, &mut dialog.baud);
                    }
                }
                SetupKind::Pair => {
                    ui.label("Pair name"); ui.text_edit_singleline(&mut dialog.pair_name);
                    ui.label("Link directory (optional)"); ui.text_edit_singleline(&mut dialog.directory);
                    ui.weak("Creates two linked raw PTYs. Existing paths are never overwritten.");
                }
                SetupKind::Bridge => {
                    let endpoints: Vec<_> = self.dock.iter_all_tabs().filter(|(_, t)| t.endpoint.state() == ConnectionState::Connected).map(|(_, t)| (t.endpoint.id().clone(), t.settings.path.clone())).collect();
                    for (label, selection) in [("A", &mut dialog.a), ("B", &mut dialog.b)] {
                        ui.label(format!("Endpoint {label}"));
                        egui::ComboBox::from_id_salt(label).width(300.0).selected_text(selection.as_ref().map(|id| id.0.trim_start_matches("serial:")).unwrap_or("Select connected terminal")).show_ui(ui, |ui| {
                            for (id, path) in &endpoints { ui.selectable_value(selection, Some(id.clone()), path); }
                        });
                    }
                    if dialog.a.is_some() && dialog.a == dialog.b { ui.colored_label(theme::ERROR, "Choose two different connected endpoints."); }
                    ui.weak("RX on A forwards to B. RX on B forwards to A.");
                }
                SetupKind::Workspace => {
                    ui.label("Workspace JSON path"); ui.text_edit_singleline(&mut dialog.workspace_path);
                    ui.weak("Loading disconnects current terminals and stops owned pairs, bridges, repeats and captures. Restored terminals stay disconnected.");
                    ui.horizontal(|ui| {
                        if ui.add_enabled(self.config_recoverable, theme::primary_button("Save to file")).clicked() { action = 2; }
                        if ui.button("Load workspace").clicked() { action = 3; }
                    });
                }
            }
            if let Some(error) = &dialog.error { ui.colored_label(theme::ERROR, error); }
            ui.separator();
            ui.horizontal(|ui| {
                if dialog.kind != SetupKind::Workspace && ui.add(theme::primary_button(match dialog.kind { SetupKind::Port => "Open port", SetupKind::Pair => "Create pair", _ => "Start bridge" })).clicked() { action = 1; }
                if ui.button("Cancel").clicked() { action = -1; }
                ui.weak("Ctrl+Enter confirms · Esc cancels");
            });
            if dialog.kind != SetupKind::Workspace && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Enter)) { action = 1; }
        });
        if action == -1 || (action == 0 && response.should_close()) {
            return;
        }
        if action == 1 {
            self.error = None;
            match dialog.kind {
                SetupKind::Port => {
                    if let Err(error) = dialog.baud.validate().and_then(|_| {
                        if dialog.settings.path.trim().is_empty() {
                            Err("Choose a device path.")
                        } else {
                            Ok(())
                        }
                    }) {
                        dialog.error = Some(error.into());
                        self.setup = Some(dialog);
                        return;
                    }
                    self.settings = dialog.settings.clone();
                    self.baud_control = BaudControl::new(self.settings.baud);
                    self.connect();
                }
                SetupKind::Pair => {
                    self.pair_name = dialog.pair_name.clone();
                    self.pair_directory = dialog.directory.clone();
                    self.create_pair();
                }
                SetupKind::Bridge => {
                    if dialog.a.is_none() || dialog.b.is_none() || dialog.a == dialog.b {
                        dialog.error = Some("Choose two different connected endpoints.".into());
                        self.setup = Some(dialog);
                        return;
                    }
                    self.bridge_a = dialog.a.clone();
                    self.bridge_b = dialog.b.clone();
                    self.start_bridge();
                }
                SetupKind::Workspace => {}
            }
            if self.error.is_none() {
                return;
            }
            dialog.error = self.error.clone();
        } else if action == 2 {
            self.save_workspace();
            match self
                .config
                .save_to(std::path::Path::new(&dialog.workspace_path))
            {
                Ok(()) => {
                    self.error = Some("Workspace exported.".into());
                    return;
                }
                Err(error) => dialog.error = Some(error),
            }
        } else if action == 3 {
            match std::fs::read_to_string(&dialog.workspace_path)
                .map_err(|e| e.to_string())
                .and_then(|text| WorkspaceConfig::parse(&text))
            {
                Ok(config) => {
                    self.bridges.clear();
                    for (_, tab) in self.dock.iter_all_tabs_mut() {
                        tab.endpoint.disconnect();
                    }
                    self.pairs.clear();
                    self.dock = DockState::new(Vec::new());
                    self.selected = None;
                    self.config = config;
                    self.restore_workspace();
                    self.config_recoverable = true;
                    self.error = None;
                    return;
                }
                Err(error) => dialog.error = Some(error),
            }
        }
        self.setup = Some(dialog);
    }
}

fn serial_form(ui: &mut egui::Ui, settings: &mut SerialSettings, baud: &mut BaudControl) {
    ui.label("Device path");
    ui.text_edit_singleline(&mut settings.path);
    ui.weak("Serial device or existing /dev/pts/N path");
    ui.horizontal_wrapped(|ui| baud.ui(ui, &mut settings.baud));
    ui.horizontal_wrapped(|ui| {
        egui::ComboBox::from_id_salt("setup-bits")
            .selected_text(format!("{} data bits", settings.data_bits))
            .show_ui(ui, |ui| {
                for bits in 5..=8 {
                    ui.selectable_value(&mut settings.data_bits, bits, bits.to_string());
                }
            });
        egui::ComboBox::from_id_salt("setup-parity")
            .selected_text(format!("Parity: {:?}", settings.parity))
            .show_ui(ui, |ui| {
                for parity in [Parity::None, Parity::Odd, Parity::Even] {
                    ui.selectable_value(&mut settings.parity, parity, format!("{parity:?}"));
                }
            });
        egui::ComboBox::from_id_salt("setup-stop")
            .selected_text(format!("{} stop bits", settings.stop_bits))
            .show_ui(ui, |ui| {
                for bits in 1..=2 {
                    ui.selectable_value(&mut settings.stop_bits, bits, bits.to_string());
                }
            });
        egui::ComboBox::from_id_salt("setup-flow")
            .selected_text(format!("Flow: {:?}", settings.flow))
            .show_ui(ui, |ui| {
                for flow in [
                    FlowControl::None,
                    FlowControl::Hardware,
                    FlowControl::Software,
                ] {
                    ui.selectable_value(&mut settings.flow, flow, format!("{flow:?}"));
                }
            });
    });
}
