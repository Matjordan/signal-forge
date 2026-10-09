mod analysis_ui;
mod baud_ui;
mod replay_ui;
mod session_ui;
mod trigger_ui;
use baud_ui::BaudControl;

mod connection_ui;
mod preset_ui;
mod terminal_ui;
mod theme;
mod update_ui;
mod workbench_ui;
mod workspace_ui;

use eframe::egui::{self, RichText};
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use signal_forge::{
    config::{FlowControl, Parity, SerialSettings, WorkspaceConfig},
    endpoint::{ConnectionState, Endpoint, EndpointError, EndpointId},
    presets::{Preset, PresetLibrary},
    repeat::{RepeatHandle, RepeatSpec},
    send::{self, Encoding, LineEnding},
    send_history::{SendEntry, SendHistory},
    serial,
    terminal_display::{self, LineDelimiter, LineDisplay, ReceiveMode, SerialFraming},
    traffic::{self, Direction, TrafficBus, TrafficEvent},
};
use std::{
    collections::VecDeque,
    sync::{mpsc::Receiver, Arc},
    time::{Duration, UNIX_EPOCH},
};

const HISTORY_LIMIT: usize = 2000;

struct Terminal {
    endpoint: Box<dyn Endpoint>,
    settings: SerialSettings,
    baud_control: BaudControl,
    show_settings: bool,
    tool: terminal_ui::TerminalTool,
    history: VecDeque<Arc<TrafficEvent>>,
    history_framing: VecDeque<SerialFraming>,
    active_framing: SerialFraming,
    rx_gap: bool,
    rx_breaks: std::collections::HashSet<u64>,
    paused: bool,
    auto_scroll: bool,
    timestamps: bool,
    analysis: signal_forge::traffic_analysis::ViewSettings,
    statistics: signal_forge::traffic_analysis::Statistics,
    view: analysis_ui::ViewCache,
    show_controls: bool,
    selection: signal_forge::terminal_selection::Selection,
    receive_mode: ReceiveMode,
    lines: LineDisplay,
    input: String,
    send_history: SendHistory,
    encoding: Encoding,
    escapes: bool,
    ending: LineEnding,
    error: Option<String>,
    repeat: Option<RepeatHandle>,
    repeat_interval_ms: u64,
    repeat_count: u64,
    continuous: bool,
    trigger: trigger_ui::TriggerUi,
    file_path: String,
    file_dialog: Option<(egui_file_dialog::FileDialog, bool)>,
    file_mode: signal_forge::file_transfer::FileMode,
    file_handle: Option<signal_forge::file_transfer::FileTransferHandle>,
    file_chunk: usize,
    file_delay_ms: u64,
    recording_mode: signal_forge::file_transfer::FileMode,
    recording_path: String,
    recording: Option<signal_forge::raw_recording::RawRecording>,
    rx_bytes: u64,
    tx_bytes: u64,
}
impl Terminal {
    fn new(endpoint: impl Endpoint + 'static, settings: SerialSettings) -> Self {
        Self {
            endpoint: Box::new(endpoint),
            baud_control: BaudControl::new(settings.baud),
            show_settings: false,
            tool: terminal_ui::TerminalTool::Send,
            active_framing: SerialFraming::from(&settings),
            settings,
            history: VecDeque::new(),
            history_framing: VecDeque::new(),
            rx_gap: false,
            rx_breaks: std::collections::HashSet::new(),
            paused: false,
            auto_scroll: true,
            timestamps: true,
            analysis: Default::default(),
            statistics: Default::default(),
            view: Default::default(),
            show_controls: false,
            selection: Default::default(),
            receive_mode: ReceiveMode::Line,
            lines: LineDisplay::default(),
            input: String::new(),
            send_history: SendHistory::default(),
            encoding: Encoding::Text,
            escapes: true,
            ending: LineEnding::None,
            error: None,
            repeat: None,
            repeat_interval_ms: 1000,
            repeat_count: 10,
            continuous: false,
            trigger: Default::default(),
            file_path: String::new(),
            file_dialog: None,
            file_mode: signal_forge::file_transfer::FileMode::Raw,
            file_handle: None,
            file_chunk: 4096,
            file_delay_ms: 0,
            recording_mode: signal_forge::file_transfer::FileMode::Raw,
            recording_path: String::new(),
            recording: None,
            rx_bytes: 0,
            tx_bytes: 0,
        }
    }
    fn receive(&mut self, event: Arc<TrafficEvent>) {
        self.statistics
            .observe(&event, std::time::Instant::now(), self.lines.delimiter);
        self.view.dirty = true;
        match event.direction {
            Direction::Rx => self.rx_bytes += event.bytes.len() as u64,
            Direction::Tx => self.tx_bytes += event.bytes.len() as u64,
        }
        if self.paused {
            if event.direction == Direction::Rx {
                if self.lines.pending.is_some() {
                    self.selection.clear();
                }
                self.lines.discard_pending();
                self.rx_gap = true;
            }
            return;
        }
        if event.direction == Direction::Rx && self.rx_gap {
            if self.lines.pending.is_some() {
                self.selection.clear();
            }
            self.lines.discard_pending();
            self.rx_breaks.insert(event.sequence);
            self.rx_gap = false;
        }
        self.lines.receive_with_framing(&event, self.active_framing);
        if self.history.len() == HISTORY_LIMIT {
            self.history_framing.pop_front();
            if let Some(old) = self.history.pop_front() {
                self.rx_breaks.remove(&old.sequence);
            }
        }
        self.history_framing.push_back(self.active_framing);
        self.history.push_back(event);
    }
    fn rebuild_lines(&mut self) {
        self.view.dirty = true;
        self.view.current = None;
        self.view.scroll_to = None;
        self.selection.clear();
        self.lines.clear();
        for (event, framing) in self.history.iter().zip(&self.history_framing) {
            if self.rx_breaks.contains(&event.sequence) {
                self.lines.discard_pending();
            }
            self.lines.receive_with_framing(event, *framing);
        }
    }
    fn send(&mut self) {
        self.error = match send::encode(&self.input, self.encoding, self.escapes, self.ending) {
            Ok(bytes) if bytes.is_empty() => Some("Enter a payload or choose a line ending".into()),
            Ok(bytes) => match self.endpoint.send(bytes) {
                Ok(()) => {
                    self.remember_input();
                    None
                }
                Err(error) => Some(error.to_string()),
            },
            Err(e) => Some(e.to_string()),
        };
    }
    fn input_entry(&self) -> SendEntry {
        SendEntry {
            input: self.input.clone(),
            encoding: self.encoding,
            escapes: self.escapes,
            ending: self.ending,
        }
    }
    fn remember_input(&mut self) {
        self.send_history.remember(self.input_entry());
    }
    fn restore_input(&mut self, entry: SendEntry) {
        self.input = entry.input;
        self.encoding = entry.encoding;
        self.escapes = entry.escapes;
        self.ending = entry.ending;
        self.error = None;
    }
    fn history_older(&mut self) {
        if let Some(entry) = self.send_history.older(self.input_entry()) {
            self.restore_input(entry);
        }
    }
    fn history_newer(&mut self) {
        if let Some(entry) = self.send_history.newer() {
            self.restore_input(entry);
        }
    }
    fn start_repeat(&mut self) {
        self.error = match send::encode(&self.input, self.encoding, self.escapes, self.ending) {
            Ok(bytes) => {
                let spec = RepeatSpec {
                    interval: Duration::from_millis(self.repeat_interval_ms),
                    count: if self.continuous {
                        None
                    } else {
                        Some(self.repeat_count)
                    },
                };
                match self.endpoint.start_repeat(bytes, spec) {
                    Ok(handle) => {
                        self.repeat = Some(handle);
                        self.remember_input();
                        None
                    }
                    Err(error) => Some(error.to_string()),
                }
            }
            Err(error) => Some(error.to_string()),
        };
    }
    fn stop_repeat(&self) {
        if let Some(handle) = &self.repeat {
            handle.cancel();
        }
    }
}

struct TerminalViewer<'a> {
    bus: &'a TrafficBus,
    artifacts: &'a mut Vec<(std::path::PathBuf, signal_forge::session::ArtifactKind)>,
    selected: &'a mut Option<EndpointId>,
    known_ports: &'a mut Vec<SerialSettings>,
    presets: &'a [Preset],
    preset_request: &'a mut Option<(EndpointId, Preset)>,
    preset_library_open: &'a mut bool,
}
impl TabViewer for TerminalViewer<'_> {
    type Tab = Terminal;
    fn title(&mut self, tab: &mut Terminal) -> egui::WidgetText {
        let color = if tab.endpoint.state() == ConnectionState::Connected {
            theme::CONNECTED
        } else {
            theme::MUTED
        };
        RichText::new(format!("{}", tab.endpoint.display_name()))
            .color(color)
            .into()
    }
    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Terminal) {
        self.selection(ui, tab);
        self.connection_status(ui, tab);
        if tab.show_settings {
            self.serial_settings(ui, tab);
        }
        self.display_controls(ui, tab);
        ui.separator();
        self.traffic_canvas(ui, tab);
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt(("terminal-tools", tab.endpoint.id().0.clone()))
            .max_height(ui.available_height().max(30.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                self.tool_strip(ui, tab);
                self.send_panel(ui, tab);
                match tab.tool {
                    terminal_ui::TerminalTool::Send => {}
                    terminal_ui::TerminalTool::Repeat => self.repeat_panel(ui, tab),
                    terminal_ui::TerminalTool::Presets => self.preset_panel(ui, tab),
                    terminal_ui::TerminalTool::Files => self.files_panel(ui, tab),
                    terminal_ui::TerminalTool::Capture => self.trigger_panel(ui, tab),
                }
                self.status_footer(ui, tab);
            });
    }
    fn on_close(&mut self, tab: &mut Terminal) -> bool {
        self.known_ports
            .retain(|settings| settings.path != tab.settings.path);
        self.known_ports.push(tab.settings.clone());
        tab.stop_repeat();
        tab.endpoint.disconnect();
        true
    }
}

pub struct Workbench {
    session: Option<signal_forge::session::Session>,
    session_ui: session_ui::SessionUi,
    pending_artifacts: Vec<(std::path::PathBuf, signal_forge::session::ArtifactKind)>,
    updater: update_ui::UpdateController,
    setup: Option<workbench_ui::SetupDialog>,
    replay_setup: Option<replay_ui::ReplaySetup>,
    preset_library_open: bool,
    active_bridge: usize,
    pairs: Vec<signal_forge::virtual_pair::VirtualPair>,
    pair_name: String,
    pair_directory: String,
    bridges: Vec<connection_ui::BridgeView>,
    bridge_a: Option<EndpointId>,
    bridge_b: Option<EndpointId>,
    library: PresetLibrary,
    library_ok: bool,
    profile_index: usize,
    new_profile: String,
    profile_file: String,
    preset_editor_open: bool,
    editor_profile: usize,
    editor_index: Option<usize>,
    preset_draft: Preset,
    selected: Option<EndpointId>,
    dock: DockState<Terminal>,
    bus: TrafficBus,
    traffic: Receiver<Arc<TrafficEvent>>,
    ports: Vec<String>,
    settings: SerialSettings,
    baud_control: BaudControl,
    config: WorkspaceConfig,
    error: Option<String>,
    config_recoverable: bool,
}
impl Workbench {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        initial_ports: Vec<String>,
        update_enabled: bool,
        restart: update_ui::RestartRequest,
        update_failure: Option<String>,
    ) -> Self {
        theme::apply(&cc.egui_ctx);
        let (config, mut error, config_recoverable) = match WorkspaceConfig::load() {
            Ok(c) => (c, None, true),
            Err(e) => (WorkspaceConfig::default(), Some(e), false),
        };
        let (library, library_ok) = match PresetLibrary::load() {
            Ok(library) => (library, true),
            Err(message) => {
                error = Some(message);
                (PresetLibrary::default(), false)
            }
        };
        let settings = config.ports.first().cloned().unwrap_or_default();
        let bus = TrafficBus::default();
        let traffic = bus.subscribe(4096);
        let mut app = Self {
            session: None,
            session_ui: Default::default(),
            pending_artifacts: Vec::new(),
            updater: update_ui::UpdateController::new(update_enabled, restart, update_failure),
            setup: None,
            replay_setup: None,
            preset_library_open: false,
            active_bridge: 0,
            pairs: Vec::new(),
            pair_name: "bench".into(),
            pair_directory: String::new(),
            bridges: Vec::new(),
            bridge_a: None,
            bridge_b: None,
            library,
            library_ok,
            profile_index: 0,
            new_profile: String::new(),
            profile_file: String::new(),
            preset_editor_open: false,
            editor_profile: 0,
            editor_index: None,
            preset_draft: Preset::default(),
            selected: None,
            dock: DockState::new(Vec::new()),
            bus,
            traffic,
            ports: Vec::new(),
            baud_control: BaudControl::new(settings.baud),
            settings,
            config,
            error,
            config_recoverable,
        };
        app.restore_workspace();
        app.refresh();
        for path in initial_ports {
            app.settings = app
                .config
                .ports
                .iter()
                .find(|settings| settings.path == path)
                .cloned()
                .unwrap_or_else(|| SerialSettings {
                    path,
                    ..Default::default()
                });
            app.baud_control = BaudControl::new(app.settings.baud);
            app.connect();
        }
        app
    }
    fn refresh(&mut self) {
        match serial::discover() {
            Ok(ports) => self.ports = ports,
            Err(error) => self.error = Some(format!("Device discovery: {error}")),
        }
    }
    fn connect(&mut self) {
        if let Err(error) = self.baud_control.validate() {
            self.error = Some(error.into());
            return;
        }
        if let Some((_, tab)) = self
            .dock
            .iter_all_tabs_mut()
            .find(|(_, tab)| tab.settings.path == self.settings.path)
        {
            if matches!(
                tab.endpoint.state(),
                ConnectionState::Connected | ConnectionState::Connecting
            ) {
                self.error = Some(format!("{} already connected", self.settings.path));
                return;
            }
            tab.settings = self.settings.clone();
            tab.baud_control = BaudControl::new(tab.settings.baud);
            tab.endpoint.disconnect();
            match signal_forge::endpoint::open(&tab.settings, self.bus.clone()) {
                Ok(endpoint) => {
                    tab.endpoint = endpoint;
                    tab.error = None;
                    self.selected = Some(tab.endpoint.id().clone());
                    self.error = None;
                }
                Err(error) => self.error = Some(format!("{}: {error}", tab.settings.path)),
            }
            return;
        }
        match signal_forge::endpoint::open(&self.settings, self.bus.clone()) {
            Ok(endpoint) => {
                self.config.ports.retain(|s| s.path != self.settings.path);
                self.config.ports.push(self.settings.clone());
                self.selected = Some(endpoint.id().clone());
                let mut tab = Terminal::new(endpoint, self.settings.clone());
                if let Some(session) = &self.session {
                    let index = self.dock.iter_all_tabs().count() + 1;
                    tab.recording_path = session
                        .suggested_path(&format!("rx-{index}"), "bin")
                        .display()
                        .to_string();
                    tab.trigger
                        .set_path(session.suggested_path(&format!("trigger-{index}"), "jsonl"));
                }
                if self.dock.iter_all_tabs().count() == 1 {
                    self.dock
                        .main_surface_mut()
                        .split_right(NodeIndex::root(), 0.5, vec![tab]);
                } else {
                    self.dock.push_to_focused_leaf(tab);
                }
                self.error = None;
            }
            Err(error) => self.error = Some(format!("{}: {error}", self.settings.path)),
        }
    }
}
impl eframe::App for Workbench {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|input| input.viewport().close_requested()) && self.session.is_some() {
            self.finish_session_outputs();
            self.snapshot_workspace();
            if let Err(error) = self.save_session_context(true) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.session_ui.open = true;
                self.session_ui.error = Some(format!("Session could not be finalized: {error}"));
            }
        }

        for _ in 0..2048 {
            let Ok(event) = self.traffic.try_recv() else {
                break;
            };
            for (_, tab) in self.dock.iter_all_tabs_mut() {
                if tab.endpoint.id() == &event.endpoint {
                    tab.receive(event.clone());
                }
            }
        }
        for (_, tab) in self.dock.iter_all_tabs() {
            if !matches!(
                tab.endpoint.state(),
                ConnectionState::Connected | ConnectionState::Connecting
            ) {
                tab.trigger.stop();
                if let Some(recording) = &tab.recording {
                    recording.stop();
                }
            }
        }
        for (_, tab) in self.dock.iter_all_tabs_mut() {
            tab.trigger.picker(ctx);
            if let Some((dialog, recording)) = &mut tab.file_dialog {
                dialog.update(ctx);
                if let Some(path) = dialog.take_picked() {
                    match path.into_os_string().into_string() {
                        Ok(path) => {
                            if *recording {
                                tab.recording_path = path;
                            } else {
                                tab.file_path = path;
                            }
                            tab.error = None;
                        }
                        Err(_) => tab.error = Some("The selected path is not valid UTF-8".into()),
                    }
                }
            }
        }
        ctx.request_repaint_after(Duration::from_millis(33));
        self.toolbar_ui(ctx);
        self.replay_setup_ui(ctx);
        self.session_panel(ctx);
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let connected = self
                    .dock
                    .iter_all_tabs()
                    .filter(|(_, t)| t.endpoint.state() == ConnectionState::Connected)
                    .count();
                ui.label(format!(
                    "{connected} connected  ·  {} detected",
                    self.ports.len()
                ));
                ui.label(format!("History: {HISTORY_LIMIT} rows per terminal"));
                let dropped = self.bus.dropped_events();
                if dropped > 0 {
                    ui.colored_label(theme::WARNING, format!("{dropped} monitor events dropped"));
                }
            });
            if let Some(error) = &self.error {
                ui.colored_label(theme::ERROR, error);
            }
            if !self.config_recoverable && ui.button("Back up original and reset workspace").clicked() {
                match WorkspaceConfig::backup_for_recovery() {
                    Ok(path) => { self.config_recoverable = true; self.error = Some(format!("Original workspace preserved at {}. Save to write the current workspace.", path.display())); }
                    Err(error) => self.error = Some(error),
                }
            }
        });
        self.workbench_sidebar(ctx);
        self.preset_editor(ctx);
        if self.setup.is_none() {
            self.preset_shortcuts(ctx);
        }
        self.connection_shortcuts(ctx);
        self.workspace_shortcuts(ctx);
        self.bridge_monitors(ctx);
        let presets = self.library.profiles[self.profile_index].presets.clone();
        let mut preset_request = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(&ctx.style()).fill(theme::BACKGROUND))
            .show(ctx, |ui| {
                if self.dock.iter_all_tabs().count() == 0 {
                    ui.vertical_centered(|ui| {
                        ui.add_space(100.0);
                        ui.label(RichText::new("Your serial workspace").size(28.0).strong());
                        ui.label("Use + New Port to configure a serial device, or Virtual Pair to create linked endpoints.");
                        if ui.add(theme::primary_button("+ Open a port")).clicked() { self.open_setup(workbench_ui::SetupKind::Port); }
                        ui.label("Open a second device to start with two panes side-by-side.");
                        ui.add_space(16.0);
                        ui.colored_label(
                            theme::ACCENT,
                            "Text + hex  ·  Explicit line endings  ·  Multiple devices",
                        );
                    });
                } else {
                    DockArea::new(&mut self.dock).show_inside(
                        ui,
                        &mut TerminalViewer {
                            bus: &self.bus,
                            artifacts: &mut self.pending_artifacts,
                            selected: &mut self.selected,
                            known_ports: &mut self.config.ports,
                            presets: &presets,
                            preset_request: &mut preset_request,
                            preset_library_open: &mut self.preset_library_open,
                        },
                    );
                }
            });
        if let Some((id, preset)) = preset_request {
            self.selected = Some(id);
            self.dispatch_preset(preset);
        }
        self.preset_library_ui(ctx);
        self.setup_dialog_ui(ctx);
        self.update_prompt(ctx);
        self.collect_session_artifacts();
    }
}
