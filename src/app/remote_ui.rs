//! Saved-host browsing and explicit, disconnected remote configuration edits.
use super::*;
use signal_forge::ssh_serial::{Discovery, RemoteDevice, SavedSshHost, SshHost};
#[derive(Default)]
pub(super) struct HostBrowser {
    discovery: Option<Discovery>,
    devices: Vec<RemoteDevice>,
    error: Option<String>,
    loaded: bool,
}
pub(super) enum RemoteEdit {
    Host {
        original: String,
        saved: SavedSshHost,
        error: Option<String>,
    },
    Endpoint {
        id: EndpointId,
        settings: SerialSettings,
        baud: BaudControl,
        device: String,
        name: String,
        error: Option<String>,
    },
}
impl Terminal {
    pub(super) fn label(&self) -> &str {
        self.display_name
            .as_deref()
            .unwrap_or_else(|| self.endpoint.display_name())
    }
    pub(super) fn open_endpoint(
        &self,
        bus: TrafficBus,
    ) -> Result<Box<dyn Endpoint>, EndpointError> {
        if self.settings.path.starts_with("ssh://") {
            let host = self
                .ssh_connection
                .clone()
                .unwrap_or(SshHost::parse(&self.settings.path)?.0);
            Ok(Box::new(
                signal_forge::ssh_serial::SshSerialEndpoint::open_identified(
                    &self.settings,
                    bus,
                    self.endpoint.id().clone(),
                    host,
                )?,
            ))
        } else {
            signal_forge::endpoint::open(&self.settings, bus)
        }
    }
    fn apply_remote_settings(
        &mut self,
        saved: &signal_forge::workspace::SavedTerminal,
        host: Option<SshHost>,
    ) {
        self.stop_repeat();
        self.endpoint.disconnect();
        self.endpoint = Box::new(super::workspace_ui::RestoredEndpoint {
            id: self.endpoint.id().clone(),
            path: saved.settings.path.clone(),
            name: saved.settings.path.clone(),
        });
        self.settings = saved.settings.clone();
        self.baud_control = BaudControl::new(self.settings.baud);
        self.remote_host = saved.remote_host.clone();
        self.ssh_connection = host;
        self.display_name = saved.display_name.clone();
        self.lines.seal_pending();
        self.rx_gap = true;
        self.preserve_rx_gap = true;
        self.view.dirty = true;
        self.error =
            Some("Configuration saved. Reconnect explicitly to use the new settings.".into());
    }
}
impl Workbench {
    pub(super) fn connection_for(&self, path: &str) -> Option<SavedSshHost> {
        let (_, device) = SshHost::parse(path).ok()?;
        self.config
            .remote_hosts
            .iter()
            .find(|host| host.connection.uri(&device) == path)
            .cloned()
    }
    pub(super) fn edit_remote_endpoint(&mut self, id: EndpointId) {
        if let Some((_, tab)) = self
            .dock
            .iter_all_tabs()
            .find(|(_, tab)| tab.endpoint.id() == &id)
        {
            if let Ok((_, device)) = SshHost::parse(&tab.settings.path) {
                self.remote_edit = Some(RemoteEdit::Endpoint {
                    id,
                    settings: tab.settings.clone(),
                    baud: BaudControl::new(tab.settings.baud),
                    device,
                    name: tab.display_name.clone().unwrap_or_default(),
                    error: None,
                });
            }
        }
    }
    pub(super) fn apply_host_edit(&mut self, original: &str, replacement: SavedSshHost) {
        if !self.config_recoverable {
            self.error = Some("Recover the workspace before saving remote edits.".into());
            return;
        }
        self.snapshot_workspace();
        match signal_forge::remote_config::edit_host(&self.config, original, replacement.clone()) {
            Err(error) => self.error = Some(error),
            Ok(config) => {
                // Save the validated configuration before disconnecting any live endpoint.
                if let Err(error) = config.save() {
                    self.error = Some(error);
                    return;
                }
                let mut saved = Vec::new();
                let mut layout = config.layout.clone();
                if let Some(layout) = &mut layout {
                    signal_forge::remote_config::terminals_mut(layout, &mut |tab| {
                        saved.push(tab.clone())
                    });
                }
                for mut window in config.windows.clone() {
                    signal_forge::remote_config::terminals_mut(&mut window.layout, &mut |tab| {
                        saved.push(tab.clone())
                    });
                }
                for (_, tab) in self.dock.iter_all_tabs_mut() {
                    if let Some(saved) = saved.iter().find(|saved| {
                        saved.endpoint_id.as_deref() == Some(&tab.endpoint.id().0)
                            && saved.remote_host.as_deref() == Some(&replacement.name)
                    }) {
                        if self.settings.path == tab.settings.path {
                            self.settings = saved.settings.clone();
                        }
                        tab.apply_remote_settings(saved, Some(replacement.connection.clone()));
                    }
                }
                self.remote_browser.remove(original);
                self.remote_browser.remove(&replacement.name);
                self.config = config;
                self.error = None;
            }
        }
    }
    pub(super) fn remote_hosts_ui(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.heading("Remote hosts");
        if self.config.remote_hosts.is_empty() {
            ui.weak("Save an SSH host in + New Port to browse its ports here.");
        }
        for host in self.config.remote_hosts.clone() {
            let connected = self
                .dock
                .iter_all_tabs()
                .filter(|(_, tab)| {
                    tab.remote_host.as_deref() == Some(&host.name)
                        && tab.endpoint.state() == ConnectionState::Connected
                })
                .count();
            let browser = self.remote_browser.entry(host.name.clone()).or_default();
            if let Some(result) = browser
                .discovery
                .as_ref()
                .and_then(|worker| worker.try_result())
            {
                browser.discovery = None;
                browser.loaded = true;
                match result {
                    Ok(devices) => {
                        browser.devices = devices;
                        browser.error = None;
                    }
                    Err(error) => {
                        browser.error = Some(error);
                    }
                }
            }
            let mut open = None;
            let mut edit = false;
            let response=egui::CollapsingHeader::new(format!("{} · {} connected",host.name,connected)).id_salt(("remote-host",&host.name)).show(ui,|ui| {
                ui.small(format!("{}{}",host.connection.host,host.connection.port.map(|port|format!(":{port}")).unwrap_or_default()));
                ui.horizontal(|ui| {
                    if super::theme::control(ui,&format!("Refresh host {}",host.name),egui::Button::new("Refresh ports")).clicked() {
                        browser.discovery=Some(Discovery::start(host.connection.clone()));browser.error=None;
                    }
                    edit=super::theme::control(ui,&format!("Edit host {}",host.name),egui::Button::new("Edit Host")).clicked();
                });
                if browser.discovery.is_some() {ui.label("Connecting / discovering…");ui.ctx().request_repaint_after(Duration::from_millis(50));}
                if let Some(error)=&browser.error {ui.colored_label(theme::ERROR,error);ui.weak("Check host address, SSH key/agent, known_hosts and remote Python 3; then Refresh.");}
                if browser.loaded && browser.discovery.is_none() && browser.error.is_none() && browser.devices.is_empty() {ui.label("No serial ports found.");}
                for device in &browser.devices {
                    let existing=self.dock.iter_all_tabs().find(|(_,tab)|SshHost::parse(&tab.settings.path).is_ok_and(|(_,path)|host.connection.uri(&path)==tab.settings.path && (path==device.path || path==device.target)));
                    let is_open=existing.as_ref().is_some_and(|(_,tab)|matches!(tab.endpoint.state(),ConnectionState::Connected|ConnectionState::Connecting));
                    ui.label(&device.path);
                    if device.path!=device.target {ui.small(format!("Device: {}",device.target));}
                    if super::theme::control(ui,&format!("Open remote {} {}",host.name,device.path),egui::Button::new(if is_open {"Focus open port"} else {"Open"})).clicked() {
                        open=Some((device.clone(),existing.map(|(_,tab)|tab.endpoint.id().clone())));
                    }
                }
            });
            super::theme::trace_control(
                &response.header_response,
                &format!("Remote host {}", host.name),
            );
            if response.body_returned.is_some() && !browser.loaded && browser.discovery.is_none() {
                browser.discovery = Some(Discovery::start(host.connection.clone()));
                ui.ctx().request_repaint();
            }
            if edit {
                self.remote_edit = Some(RemoteEdit::Host {
                    original: host.name.clone(),
                    saved: host.clone(),
                    error: None,
                });
            }
            if let Some((device, existing)) = open {
                if let Some(id) = existing {
                    self.select_terminal(id.clone());
                    if let Some((_, tab)) = self
                        .dock
                        .iter_all_tabs()
                        .find(|(_, tab)| tab.endpoint.id() == &id)
                    {
                        self.settings = tab.settings.clone();
                    }
                } else {
                    self.settings = self
                        .config
                        .ports
                        .iter()
                        .find(|port| port.path == host.connection.uri(&device.path))
                        .cloned()
                        .unwrap_or_default();
                    self.settings.path = host.connection.uri(&device.path);
                }
                let active = self.dock.iter_all_tabs().any(|(_, tab)| {
                    tab.settings.path == self.settings.path
                        && matches!(
                            tab.endpoint.state(),
                            ConnectionState::Connected | ConnectionState::Connecting
                        )
                });
                if !active {
                    self.baud_control = BaudControl::new(self.settings.baud);
                    self.connect();
                }
            }
        }
    }
    pub(super) fn remote_editor(&mut self, ctx: &egui::Context) {
        let Some(mut edit) = self.remote_edit.take() else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("remote-edit")).show(ctx,|ui| {
            ui.set_width(410.0);
            ui.heading("Edit remote configuration");
            match &mut edit {
                RemoteEdit::Host{saved,error,..}=> {
                    ui.label("Friendly host name");super::theme::control(ui,"Remote host name",egui::TextEdit::singleline(&mut saved.name));
                    ui.label("Hostname / IP / SSH config alias");super::theme::control(ui,"Remote address",egui::TextEdit::singleline(&mut saved.connection.host));
                    let mut user=saved.connection.username.clone().unwrap_or_default();ui.label("Username (blank uses SSH config)");ui.text_edit_singleline(&mut user);saved.connection.username=(!user.is_empty()).then_some(user);
                    let mut port=saved.connection.port.unwrap_or(0);ui.horizontal(|ui|{ui.label("SSH port (0 uses config)");ui.add(egui::DragValue::new(&mut port));});saved.connection.port=(port!=0).then_some(port);
                    let mut identity=saved.connection.identity_file.clone().unwrap_or_default();ui.label("Identity file path (blank uses SSH config / agent)");ui.text_edit_singleline(&mut identity);saved.connection.identity_file=(!identity.is_empty()).then_some(identity);
                    ui.weak("Stores only a key file path, never key contents or passwords. Strict known-host verification remains enabled.");
                    if let Some(error)=error {ui.colored_label(theme::ERROR,error);}
                }
                RemoteEdit::Endpoint{settings,baud,device,name,error,..}=> {
                    ui.label("Friendly endpoint name (optional)");super::theme::control(ui,"Remote endpoint name",egui::TextEdit::singleline(name));
                    settings.path=device.clone();super::workbench_ui::serial_form(ui,settings,baud);*device=settings.path.clone();
                    ui.weak("SSH address, user and key are shared host settings. Edit Host in the sidebar to change them for all its ports.");
                    if let Some(error)=error {ui.colored_label(theme::ERROR,error);}
                }
            }
            ui.separator();ui.label("Applying saves the configuration and disconnects affected endpoints. Active sends/repeats/bridges stop. Terminal history, drafts, presets and layout are kept. Reconnect explicitly afterward.");
            ui.horizontal(|ui| {apply=super::theme::control(ui,"Apply remote edit",egui::Button::new("Save and disconnect")).clicked();cancel=ui.button("Cancel").clicked();});
        });
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            cancel = true;
        }
        if cancel {
            return;
        }
        if apply {
            match &mut edit {
                RemoteEdit::Host {
                    original,
                    saved,
                    error,
                } => {
                    self.apply_host_edit(original, saved.clone());
                    *error = self.error.clone();
                    if error.is_none() {
                        return;
                    }
                }
                RemoteEdit::Endpoint {
                    id,
                    settings,
                    baud,
                    device,
                    name,
                    error,
                } => {
                    let result = (|| {
                        baud.validate()?;
                        if name.len() > 256 || name.contains(['\0', '\n', '\r']) {
                            return Err("Invalid endpoint name".into());
                        }
                        let tab = self
                            .dock
                            .iter_all_tabs()
                            .find(|(_, tab)| tab.endpoint.id() == id)
                            .ok_or("Endpoint no longer exists")?
                            .1;
                        let host = tab.ssh_connection.clone().unwrap_or(
                            SshHost::parse(&tab.settings.path)
                                .map_err(|e| e.to_string())?
                                .0,
                        );
                        settings.path = host.uri(device);
                        settings.validate()?;
                        if self.dock.iter_all_tabs().any(|(_, tab)| {
                            tab.endpoint.id() != id && tab.settings.path == settings.path
                        }) {
                            return Err("That remote device already has a terminal".into());
                        }
                        if !self.config_recoverable {
                            return Err("Recover the workspace before saving remote edits.".into());
                        }
                        let mut saved = tab.saved();
                        saved.settings = settings.clone();
                        saved.endpoint_id = Some(id.0.clone());
                        saved.display_name = (!name.is_empty()).then(|| name.clone());
                        self.snapshot_workspace();
                        let mut config = self.config.clone();
                        let old_path =
                            tab_path(&config, id).ok_or("Endpoint missing from workspace")?;
                        let mut update = |tab: &mut signal_forge::workspace::SavedTerminal| {
                            if tab
                                .endpoint_id
                                .as_deref()
                                .unwrap_or(&format!("serial:{}", tab.settings.path))
                                == id.0
                            {
                                *tab = saved.clone();
                            }
                        };
                        if let Some(layout) = &mut config.layout {
                            signal_forge::remote_config::terminals_mut(layout, &mut update);
                        }
                        for window in &mut config.windows {
                            signal_forge::remote_config::terminals_mut(
                                &mut window.layout,
                                &mut update,
                            );
                        }
                        config.ports.retain(|port| port.path != old_path);
                        config.ports.push(settings.clone());
                        if config.selected.as_deref() == Some(&old_path) {
                            config.selected = Some(settings.path.clone());
                        }
                        config.validate()?;
                        config.save()?;
                        if let Some((_, tab)) = self
                            .dock
                            .iter_all_tabs_mut()
                            .find(|(_, tab)| tab.endpoint.id() == id)
                        {
                            tab.apply_remote_settings(&saved, Some(host));
                        }
                        if self.settings.path == old_path {
                            self.settings = settings.clone();
                            self.baud_control = BaudControl::new(settings.baud);
                        }
                        self.config = config;
                        Ok::<_, String>(())
                    })();
                    match result {
                        Ok(()) => return,
                        Err(message) => *error = Some(message),
                    }
                }
            }
        }
        self.remote_edit = Some(edit);
    }
}
fn tab_path(config: &WorkspaceConfig, id: &EndpointId) -> Option<String> {
    let mut found = None;
    let mut visit = |tab: &mut signal_forge::workspace::SavedTerminal| {
        if tab
            .endpoint_id
            .clone()
            .unwrap_or_else(|| format!("serial:{}", tab.settings.path))
            == id.0
        {
            found = Some(tab.settings.path.clone());
        }
    };
    if let Some(mut layout) = config.layout.clone() {
        signal_forge::remote_config::terminals_mut(&mut layout, &mut visit);
    }
    for mut window in config.windows.clone() {
        signal_forge::remote_config::terminals_mut(&mut window.layout, &mut visit);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_edit_preserves_draft_partial_history_and_identity_through_rebuild() {
        let mut saved = signal_forge::workspace::SavedTerminal {
            settings: SerialSettings {
                path: "ssh://old/dev/ttyUSB0".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut tab = Terminal::restored(&saved);
        let id = tab.endpoint.id().clone();
        tab.input = "unsent draft".into();
        let event = |sequence, bytes: &[u8]| {
            Arc::new(TrafficEvent {
                endpoint: id.clone(),
                sequence,
                timestamp: std::time::SystemTime::now(),
                direction: Direction::Rx,
                bytes: bytes.to_vec().into(),
            })
        };
        tab.receive(event(1, b"BEFORE"));
        saved.settings.path = "ssh://new/dev/ttyUSB1".into();
        tab.apply_remote_settings(&saved, None);
        assert_eq!(tab.endpoint.id(), &id);
        assert_eq!(tab.input, "unsent draft");
        assert_eq!(tab.endpoint.state(), ConnectionState::Disconnected);
        assert_eq!(tab.lines.row(0).unwrap().bytes, b"BEFORE");
        assert!(tab.lines.row(0).unwrap().terminator.is_empty());
        tab.receive(event(2, b"AFTER\n"));
        tab.rebuild_lines();
        assert_eq!(tab.lines.len(), 2);
        assert_eq!(tab.lines.row(0).unwrap().bytes, b"BEFORE");
        assert_eq!(tab.lines.row(1).unwrap().bytes, b"AFTER");
        assert_ne!(
            tab.lines.row(0).unwrap().sources[0].epoch,
            tab.lines.row(1).unwrap().sources[0].epoch
        );
    }
}
