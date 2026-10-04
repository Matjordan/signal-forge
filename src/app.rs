use eframe::egui::{self, Color32, RichText};
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use signal_forge::{config::{FlowControl, Parity, SerialSettings, WorkspaceConfig}, endpoint::{ConnectionState, Endpoint}, send::{self, Encoding, LineEnding}, serial::{self, SerialEndpoint}, traffic::{self, Direction, TrafficBus, TrafficEvent}};
use std::{collections::VecDeque, sync::{mpsc::Receiver, Arc}, time::{Duration, UNIX_EPOCH}};

const HISTORY_LIMIT: usize = 2000;
const ACCENT: Color32 = Color32::from_rgb(69,208,187);

struct Terminal {
    endpoint: SerialEndpoint,
    history: VecDeque<Arc<TrafficEvent>>,
    paused: bool,
    auto_scroll: bool,
    timestamps: bool,
    hex: bool,
    input: String,
    encoding: Encoding,
    escapes: bool,
    ending: LineEnding,
    error: Option<String>,
    rx_bytes: u64,
    tx_bytes: u64,
}
impl Terminal {
    fn new(endpoint: SerialEndpoint) -> Self {
        Self { endpoint, history: VecDeque::new(), paused: false, auto_scroll: true, timestamps: true, hex: false, input: String::new(), encoding: Encoding::Text, escapes: true, ending: LineEnding::None, error: None, rx_bytes: 0, tx_bytes: 0 }
    }
    fn receive(&mut self, event: Arc<TrafficEvent>) {
        match event.direction { Direction::Rx => self.rx_bytes += event.bytes.len() as u64, Direction::Tx => self.tx_bytes += event.bytes.len() as u64 }
        if self.paused { return; }
        if self.history.len() == HISTORY_LIMIT { self.history.pop_front(); }
        self.history.push_back(event);
    }
    fn send(&mut self) {
        self.error = match send::encode(&self.input,self.encoding,self.escapes,self.ending) {
            Ok(bytes) if bytes.is_empty() => Some("Enter a payload or choose a line ending".into()),
            Ok(bytes) => self.endpoint.send(bytes).err().map(|e| e.to_string()),
            Err(e) => Some(e.to_string()),
        };
    }
}

struct TerminalViewer;
impl TabViewer for TerminalViewer {
    type Tab = Terminal;
    fn title(&mut self,tab:&mut Terminal) -> egui::WidgetText { format!("● {}",tab.endpoint.display_name()).into() }
    fn ui(&mut self,ui:&mut egui::Ui,tab:&mut Terminal) {
        ui.horizontal(|ui| {
            ui.checkbox(&mut tab.hex,"Hex");
            ui.checkbox(&mut tab.timestamps,"Timestamps");
            ui.checkbox(&mut tab.auto_scroll,"Auto-scroll");
            ui.checkbox(&mut tab.paused,"Pause display");
            if ui.button("Clear").clicked() { tab.history.clear(); }
        });
        match tab.endpoint.state() {
            ConnectionState::Connected => { ui.label(RichText::new(format!("Connected  ·  RX {} B  ·  TX {} B",tab.rx_bytes,tab.tx_bytes)).color(ACCENT)); }
            ConnectionState::Disconnected => { ui.colored_label(Color32::YELLOW,"Disconnected — close this tab and reopen the device to reconnect"); }
            ConnectionState::Fault(error) => { ui.colored_label(Color32::LIGHT_RED,format!("{}: {error}",tab.endpoint.display_name())); }
        }
        if tab.paused { ui.label("Display paused; new rows are discarded while serial I/O continues."); }
        ui.separator();
        let terminal_height = (ui.available_height()-125.0).max(80.0);
        egui::ScrollArea::both().id_salt((tab.endpoint.id().0.clone(),"traffic")).auto_shrink([false,false]).max_height(terminal_height).stick_to_bottom(tab.auto_scroll).show_rows(ui,18.0,tab.history.len(),|ui,range| {
            for index in range {
                let event = &tab.history[index];
                let color = if event.direction == Direction::Rx { Color32::from_rgb(161,214,181) } else { Color32::from_rgb(111,179,241) };
                let time = if tab.timestamps {
                    let elapsed = event.timestamp.duration_since(UNIX_EPOCH).unwrap_or_default();
                    let seconds = elapsed.as_secs()%86400;
                    format!("{:02}:{:02}:{:02}.{:03} ",seconds/3600,(seconds/60)%60,seconds%60,elapsed.subsec_millis())
                } else { String::new() };
                let payload = if tab.hex { traffic::hex(&event.bytes) } else { traffic::ascii(&event.bytes) };
                ui.label(RichText::new(format!("{time}{}  {payload}",if event.direction == Direction::Rx { "RX" } else { "TX" })).monospace().color(color));
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut tab.encoding,Encoding::Text,"Text");
            ui.selectable_value(&mut tab.encoding,Encoding::Hex,"Hex bytes");
            ui.add_enabled(tab.encoding==Encoding::Text,egui::Checkbox::new(&mut tab.escapes,"Interpret escapes"));
        });
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt((tab.endpoint.id().0.clone(),"ending")).selected_text(format!("Ending: {:?}",tab.ending)).show_ui(ui,|ui| {
                for (value,label) in [(LineEnding::None,"None"),(LineEnding::Cr,"CR"),(LineEnding::Lf,"LF"),(LineEnding::CrLf,"CRLF")] { ui.selectable_value(&mut tab.ending,value,label); }
            });
            let input = ui.add(egui::TextEdit::singleline(&mut tab.input).desired_width((ui.available_width()-70.0).max(100.0)).hint_text("Payload, e.g. AT\\r\\n"));
            let enter = input.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.add_enabled(tab.endpoint.state()==ConnectionState::Connected,egui::Button::new("Send")).clicked() || enter { tab.send(); }
        });
        if let Some(error) = &tab.error { ui.colored_label(Color32::LIGHT_RED,error); }
        ui.small("Enter sends to this tab. Closing the tab disconnects its device. Timestamps are UTC.");
    }
    fn on_close(&mut self,tab:&mut Terminal) -> bool { tab.endpoint.disconnect(); true }
}

pub struct Workbench {
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
    pub fn new(cc:&eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals.panel_fill = Color32::from_rgb(22,27,34);
        style.visuals.window_fill = Color32::from_rgb(25,31,39);
        style.visuals.selection.bg_fill = Color32::from_rgb(31,105,99);
        cc.egui_ctx.set_style(style);
        let (config,error,config_recoverable) = match WorkspaceConfig::load() { Ok(c) => (c,None,true), Err(e) => (WorkspaceConfig::default(),Some(e),false) };
        let settings = config.ports.first().cloned().unwrap_or_default();
        let bus = TrafficBus::default();
        let traffic = bus.subscribe(4096);
        let mut app = Self { dock: DockState::new(Vec::new()),bus,traffic,ports:Vec::new(),settings,config,error,config_recoverable };
        app.refresh();
        app
    }
    fn refresh(&mut self) {
        match serial::discover() { Ok(ports) => self.ports=ports, Err(error) => self.error=Some(error.to_string()) }
    }
    fn connect(&mut self) {
        if self.dock.iter_all_tabs().any(|(_,tab)| tab.endpoint.display_name()==self.settings.path) { self.error=Some("This device already has a terminal. Close its tab before reconnecting.".into()); return; }
        match SerialEndpoint::open(&self.settings,self.bus.clone()) {
            Ok(endpoint) => {
                self.config.ports.retain(|s| s.path != self.settings.path);
                self.config.ports.push(self.settings.clone());
                let tab=Terminal::new(endpoint);
                if self.dock.iter_all_tabs().count()==1 {
                    self.dock.main_surface_mut().split_right(NodeIndex::root(),0.5,vec![tab]);
                } else { self.dock.push_to_focused_leaf(tab); }
                self.error=None;
            }
            Err(error) => self.error=Some(error.to_string()),
        }
    }
    fn settings_ui(&mut self,ui:&mut egui::Ui) {
        ui.heading("Serial settings");
        ui.label("Device path");
        ui.text_edit_singleline(&mut self.settings.path);
        ui.small("Physical device or an existing /dev/pts/N path");
        ui.horizontal(|ui| { ui.label("Baud"); ui.add(egui::DragValue::new(&mut self.settings.baud).range(1..=4_000_000).speed(100)); });
        egui::ComboBox::from_id_salt("bits").selected_text(format!("{} data bits",self.settings.data_bits)).show_ui(ui,|ui| { for value in 5..=8 { ui.selectable_value(&mut self.settings.data_bits,value,value.to_string()); } });
        egui::ComboBox::from_id_salt("parity").selected_text(format!("Parity: {:?}",self.settings.parity)).show_ui(ui,|ui| { for value in [Parity::None,Parity::Odd,Parity::Even] { ui.selectable_value(&mut self.settings.parity,value,format!("{value:?}")); } });
        egui::ComboBox::from_id_salt("stop").selected_text(format!("{} stop bits",self.settings.stop_bits)).show_ui(ui,|ui| { for value in 1..=2 { ui.selectable_value(&mut self.settings.stop_bits,value,value.to_string()); } });
        egui::ComboBox::from_id_salt("flow").selected_text(format!("Flow: {:?}",self.settings.flow)).show_ui(ui,|ui| { for value in [FlowControl::None,FlowControl::Hardware,FlowControl::Software] { ui.selectable_value(&mut self.settings.flow,value,format!("{value:?}")); } });
        if ui.add_enabled(!self.settings.path.is_empty(),egui::Button::new("Open terminal").fill(Color32::from_rgb(30,96,89))).clicked() { self.connect(); }
        ui.separator();
        ui.heading("Recent devices");
        for saved in &self.config.ports { if ui.selectable_label(saved.path==self.settings.path,&saved.path).clicked() { self.settings=saved.clone(); } }
        ui.separator();
        ui.small("Drag tabs to arrange terminals side-by-side. Open devices have independent settings.");
    }
}
impl eframe::App for Workbench {
    fn update(&mut self,ctx:&egui::Context,_frame:&mut eframe::Frame) {
        for _ in 0..2048 {
            let Ok(event) = self.traffic.try_recv() else { break; };
            for (_,tab) in self.dock.iter_all_tabs_mut() { if tab.endpoint.id()==&event.endpoint { tab.receive(event.clone()); } }
        }
        ctx.request_repaint_after(Duration::from_millis(33));
        egui::TopBottomPanel::top("header").show(ctx,|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("◈  SIGNAL FORGE").strong().size(22.0).color(ACCENT));
                ui.label("SERIAL WORKBENCH");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                    if ui.add_enabled(self.config_recoverable,egui::Button::new("Save port settings")).clicked() { self.error=self.config.save().err(); }
                    if ui.button("Refresh devices").clicked() { self.refresh(); }
                });
            });
        });
        egui::TopBottomPanel::bottom("status").show(ctx,|ui| {
            ui.horizontal(|ui| {
                let connected=self.dock.iter_all_tabs().filter(|(_,t)| t.endpoint.state()==ConnectionState::Connected).count();
                ui.label(format!("{connected} connected  ·  {} detected",self.ports.len()));
                ui.label(format!("History: {HISTORY_LIMIT} rows per terminal"));
                let dropped=self.bus.dropped_events();
                if dropped>0 { ui.colored_label(Color32::YELLOW,format!("{dropped} monitor events dropped")); }
            });
            if let Some(error)=&self.error { ui.colored_label(Color32::LIGHT_RED,error); }
        });
        egui::SidePanel::left("devices").resizable(true).default_width(240.0).show(ctx,|ui| {
            ui.heading("Devices");
            if self.ports.is_empty() { ui.label("No serial devices detected"); }
            for path in &self.ports {
                if ui.selectable_label(self.settings.path==*path,path).clicked() {
                    self.settings=self.config.ports.iter().find(|s| s.path==*path).cloned().unwrap_or_else(|| SerialSettings { path:path.clone(),..Default::default() });
                }
            }
            ui.separator();
            self.settings_ui(ui);
        });
        egui::CentralPanel::default().show(ctx,|ui| {
            if self.dock.iter_all_tabs().count()==0 {
                ui.vertical_centered(|ui| {
                    ui.add_space(100.0);
                    ui.label(RichText::new("Your serial workspace").size(28.0).strong());
                    ui.label("Choose a device, set its parameters, and open a terminal.");
                    ui.label("Open a second device to start with two panes side-by-side.");
                    ui.add_space(16.0);
                    ui.colored_label(ACCENT,"Text + hex  ·  Explicit line endings  ·  Multiple devices");
                });
            } else { DockArea::new(&mut self.dock).show_inside(ui,&mut TerminalViewer); }
        });
    }
}
