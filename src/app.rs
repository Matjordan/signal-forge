mod connection_ui;
mod preset_ui;
mod workspace_ui;

use eframe::egui::{self, Color32, RichText};
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use signal_forge::{
    config::{FlowControl, Parity, SerialSettings, WorkspaceConfig},
    endpoint::{ConnectionState, Endpoint, EndpointError, EndpointId},
    presets::{Preset, PresetLibrary},
    repeat::{RepeatHandle, RepeatSpec},
    send::{self, Encoding, LineEnding},
    send_history::{SendEntry, SendHistory},
    serial::{self, SerialEndpoint},
    traffic::{self, Direction, TrafficBus, TrafficEvent},
};
use std::{
    collections::VecDeque,
    sync::{mpsc::Receiver, Arc},
    time::{Duration, UNIX_EPOCH},
};

const HISTORY_LIMIT: usize = 2000;
const ACCENT: Color32 = Color32::from_rgb(43, 145, 246);
const GREEN: Color32 = Color32::from_rgb(89, 210, 118);

struct Terminal {
    endpoint: Box<dyn Endpoint>,
    settings: SerialSettings,
    history: VecDeque<Arc<TrafficEvent>>,
    paused: bool,
    auto_scroll: bool,
    timestamps: bool,
    hex: bool,
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
    rx_bytes: u64,
    tx_bytes: u64,
}
impl Terminal {
    fn new(endpoint: impl Endpoint + 'static, settings: SerialSettings) -> Self {
        Self {
            endpoint: Box::new(endpoint),
            settings,
            history: VecDeque::new(),
            paused: false,
            auto_scroll: true,
            timestamps: true,
            hex: false,
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
            rx_bytes: 0,
            tx_bytes: 0,
        }
    }
    fn receive(&mut self, event: Arc<TrafficEvent>) {
        match event.direction {
            Direction::Rx => self.rx_bytes += event.bytes.len() as u64,
            Direction::Tx => self.tx_bytes += event.bytes.len() as u64,
        }
        if self.paused {
            return;
        }
        if self.history.len() == HISTORY_LIMIT {
            self.history.pop_front();
        }
        self.history.push_back(event);
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
    selected: &'a mut Option<EndpointId>,
}
impl TabViewer for TerminalViewer<'_> {
    type Tab = Terminal;
    fn title(&mut self, tab: &mut Terminal) -> egui::WidgetText {
        let color = if tab.endpoint.state() == ConnectionState::Connected {
            GREEN
        } else {
            Color32::GRAY
        };
        RichText::new(format!("{}", tab.endpoint.display_name()))
            .color(color)
            .into()
    }
    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Terminal) {
        if ui.rect_contains_pointer(ui.max_rect()) && ui.input(|input| input.pointer.any_pressed())
        {
            *self.selected = Some(tab.endpoint.id().clone());
        }
        ui.push_id(tab.endpoint.id().0.clone(), |ui| {
            ui.horizontal_wrapped(|ui| {
                let connected = tab.endpoint.state() == ConnectionState::Connected;
                ui.add_enabled_ui(!connected, |ui| {
                    ui.label("Baud");
                    ui.add(
                        egui::DragValue::new(&mut tab.settings.baud)
                            .range(1..=4_000_000)
                            .speed(100),
                    );
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
                        egui::Button::new(if connected { "Disconnect" } else { "Reconnect" })
                            .fill(Color32::from_rgb(21, 99, 218)),
                    )
                    .on_hover_text("Disconnect stops repeats and attached bridges; capture finishes. Reconnect explicitly opens this device.")
                    .clicked()
                {
                    if connected {
                        tab.stop_repeat();
                        tab.endpoint.disconnect();
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
        ui.horizontal(|ui| {
            ui.selectable_value(&mut tab.hex, false, "Terminal");
            ui.selectable_value(&mut tab.hex, true, "Hex");
            ui.checkbox(&mut tab.timestamps, "Timestamps");
            ui.checkbox(&mut tab.auto_scroll, "Auto-scroll");
            ui.checkbox(&mut tab.paused, "Pause display");
            if ui.button("Clear").clicked() {
                tab.history.clear();
            }
        });
        match tab.endpoint.state() {
            ConnectionState::Connected => {
                ui.label(
                    RichText::new(format!(
                        "Connected  ·  RX {} B  ·  TX {} B",
                        tab.rx_bytes, tab.tx_bytes
                    ))
                    .color(GREEN),
                );
            }
            ConnectionState::Disconnected => {
                ui.colored_label(
                    Color32::YELLOW,
                    "Disconnected — adjust settings above and reconnect",
                );
            }
            ConnectionState::Fault(error) => {
                ui.colored_label(
                    Color32::LIGHT_RED,
                    format!("{}: {error}", tab.endpoint.display_name()),
                );
            }
        }
        if tab.paused {
            ui.label("Display paused; new rows are discarded while serial I/O continues.");
        }
        ui.separator();
        let terminal_height = (ui.available_height() - 205.0).max(80.0);
        egui::Frame::new()
            .fill(Color32::from_rgb(7, 13, 20))
            .inner_margin(6.0)
            .show(ui, |ui| {
                egui::ScrollArea::both()
                    .id_salt((tab.endpoint.id().0.clone(), "traffic"))
                    .auto_shrink([false, false])
                    .max_height(terminal_height)
                    .min_scrolled_height(terminal_height)
                    .stick_to_bottom(tab.auto_scroll)
                    .show_rows(ui, 18.0, tab.history.len(), |ui, range| {
                        for index in range {
                            let event = &tab.history[index];
                            let color = if event.direction == Direction::Rx {
                                Color32::from_rgb(128, 205, 141)
                            } else {
                                Color32::from_rgb(60, 158, 246)
                            };
                            let time = if tab.timestamps {
                                let elapsed = event
                                    .timestamp
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap_or_default();
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
                            let payload = if tab.hex {
                                traffic::hex(&event.bytes)
                            } else {
                                traffic::ascii(&event.bytes)
                            };
                            ui.add(
                                egui::Label::new(
                                    RichText::new(format!(
                                        "{time}{}  {payload}",
                                        if event.direction == Direction::Rx {
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
                    });
            });
        ui.separator();
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
                    egui::Button::new("Send").fill(Color32::from_rgb(21, 99, 218)),
                )
                .clicked();
            if clicked || (enter && focused) {
                tab.send();
            }
        });
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
                .add_enabled(
                    active,
                    egui::Button::new("Stop repeat").fill(Color32::from_rgb(140, 40, 50)),
                )
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
                    ACCENT
                } else {
                    Color32::GRAY
                }),
            );
        }
        if let Some(error) = &tab.error {
            ui.colored_label(Color32::LIGHT_RED, format!("{}: {error}", tab.settings.path));
        }
        ui.small(
            "Enter sends · Up cycles previous messages · Down returns to draft · History is per terminal.",
        );
    }
    fn on_close(&mut self, tab: &mut Terminal) -> bool {
        tab.stop_repeat();
        tab.endpoint.disconnect();
        true
    }
}

pub struct Workbench {
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
    config: WorkspaceConfig,
    error: Option<String>,
    config_recoverable: bool,
}
impl Workbench {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_ports: Vec<String>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals.panel_fill = Color32::from_rgb(17, 27, 38);
        style.visuals.window_fill = Color32::from_rgb(20, 30, 42);
        style.visuals.selection.bg_fill = Color32::from_rgb(28, 76, 128);
        cc.egui_ctx.set_style(style);
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
            app.connect();
        }
        app
    }
    fn refresh(&mut self) {
        match serial::discover() {
            Ok(ports) => self.ports = ports,
            Err(error) => self.error = Some(format!("{}: {error}", self.settings.path)),
        }
    }
    fn connect(&mut self) {
        if let Some((_, tab)) = self.dock.iter_all_tabs_mut().find(|(_, tab)| tab.settings.path == self.settings.path) {
            if tab.endpoint.state() == ConnectionState::Connected {
                self.error = Some(format!("{} already connected", self.settings.path));
                return;
            }
            match SerialEndpoint::open(&tab.settings, self.bus.clone()) {
                Ok(endpoint) => { tab.endpoint = Box::new(endpoint); tab.error = None; self.selected = Some(tab.endpoint.id().clone()); self.error = None; }
                Err(error) => self.error = Some(format!("{}: {error}", tab.settings.path)),
            }
            return;
        }
        match SerialEndpoint::open(&self.settings, self.bus.clone()) {
            Ok(endpoint) => {
                self.config.ports.retain(|s| s.path != self.settings.path);
                self.config.ports.push(self.settings.clone());
                self.selected = Some(endpoint.id().clone());
                let tab = Terminal::new(endpoint, self.settings.clone());
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
    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Serial settings");
        ui.label("Device path");
        ui.text_edit_singleline(&mut self.settings.path);
        ui.small("Physical device or an existing /dev/pts/N path");
        ui.horizontal(|ui| {
            ui.label("Baud");
            ui.add(
                egui::DragValue::new(&mut self.settings.baud)
                    .range(1..=4_000_000)
                    .speed(100),
            );
        });
        egui::ComboBox::from_id_salt("bits")
            .selected_text(format!("{} data bits", self.settings.data_bits))
            .show_ui(ui, |ui| {
                for value in 5..=8 {
                    ui.selectable_value(&mut self.settings.data_bits, value, value.to_string());
                }
            });
        egui::ComboBox::from_id_salt("parity")
            .selected_text(format!("Parity: {:?}", self.settings.parity))
            .show_ui(ui, |ui| {
                for value in [Parity::None, Parity::Odd, Parity::Even] {
                    ui.selectable_value(&mut self.settings.parity, value, format!("{value:?}"));
                }
            });
        egui::ComboBox::from_id_salt("stop")
            .selected_text(format!("{} stop bits", self.settings.stop_bits))
            .show_ui(ui, |ui| {
                for value in 1..=2 {
                    ui.selectable_value(&mut self.settings.stop_bits, value, value.to_string());
                }
            });
        egui::ComboBox::from_id_salt("flow")
            .selected_text(format!("Flow: {:?}", self.settings.flow))
            .show_ui(ui, |ui| {
                for value in [
                    FlowControl::None,
                    FlowControl::Hardware,
                    FlowControl::Software,
                ] {
                    ui.selectable_value(&mut self.settings.flow, value, format!("{value:?}"));
                }
            });
        if ui
            .add_enabled(
                !self.settings.path.is_empty(),
                egui::Button::new("New port terminal").fill(Color32::from_rgb(21, 99, 218)),
            )
            .clicked()
        {
            self.connect();
        }
        ui.separator();
        ui.heading("Recent devices");
        for saved in &self.config.ports {
            if ui
                .selectable_label(saved.path == self.settings.path, &saved.path)
                .clicked()
            {
                self.settings = saved.clone();
            }
        }
        ui.separator();
        ui.small(
            "Drag tabs to arrange terminals side-by-side. Open devices have independent settings.",
        );
    }
}
impl eframe::App for Workbench {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
        ctx.request_repaint_after(Duration::from_millis(33));
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Signal Forge")
                        .strong()
                        .size(22.0)
                        .color(ACCENT),
                );
                ui.label("Serial Workbench");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            self.config_recoverable,
                            egui::Button::new("Save workspace"),
                        )
                        .clicked()
                    {
                        self.save_workspace();
                    }
                    if ui.button("Refresh devices").clicked() {
                        self.refresh();
                    }
                });
            });
        });
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
                    ui.colored_label(Color32::YELLOW, format!("{dropped} monitor events dropped"));
                }
            });
            if let Some(error) = &self.error {
                ui.colored_label(Color32::LIGHT_RED, error);
            }
            if !self.config_recoverable && ui.button("Back up original and reset workspace").clicked() {
                match WorkspaceConfig::backup_for_recovery() {
                    Ok(path) => { self.config_recoverable = true; self.error = Some(format!("Original workspace preserved at {}. Save to write the current workspace.", path.display())); }
                    Err(error) => self.error = Some(error),
                }
            }
        });
        egui::SidePanel::left("devices")
            .resizable(true)
            .default_width(240.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("connections-sidebar")
                    .show(ui, |ui| {
                        ui.heading("Endpoints");
                        if self.ports.is_empty() {
                            ui.label("No serial devices detected");
                        }
                        egui::ScrollArea::vertical()
                            .id_salt("device_list")
                            .max_height(180.0)
                            .show(ui, |ui| {
                                for path in &self.ports {
                                    if ui
                                        .selectable_label(self.settings.path == *path, path)
                                        .clicked()
                                    {
                                        self.settings = self
                                            .config
                                            .ports
                                            .iter()
                                            .find(|s| s.path == *path)
                                            .cloned()
                                            .unwrap_or_else(|| SerialSettings {
                                                path: path.clone(),
                                                ..Default::default()
                                            });
                                    }
                                }
                            });
                        ui.separator();
                        self.settings_ui(ui);
                        self.connections_ui(ui);
                    });
            });
        egui::SidePanel::right("presets")
            .resizable(true)
            .default_width(270.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.presets_ui(ui));
            });
        self.preset_editor(ctx);
        self.preset_shortcuts(ctx);
        self.connection_shortcuts(ctx);
        self.workspace_shortcuts(ctx);
        self.bridge_monitors(ctx);
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.dock.iter_all_tabs().count() == 0 {
                ui.vertical_centered(|ui| {
                    ui.add_space(100.0);
                    ui.label(RichText::new("Your serial workspace").size(28.0).strong());
                    ui.label("Choose a device, set its parameters, and open a terminal.");
                    ui.label("Open a second device to start with two panes side-by-side.");
                    ui.add_space(16.0);
                    ui.colored_label(
                        ACCENT,
                        "Text + hex  ·  Explicit line endings  ·  Multiple devices",
                    );
                });
            } else {
                DockArea::new(&mut self.dock).show_inside(
                    ui,
                    &mut TerminalViewer {
                        bus: &self.bus,
                        selected: &mut self.selected,
                    },
                );
            }
        });
    }
}
