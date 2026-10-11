use super::*;
use signal_forge_network::Mode;
impl Workbench {
    pub(super) fn network_setup_ui(&mut self, ctx: &egui::Context) {
        let Some(mut config) = self.network_setup.take() else {
            return;
        };
        let mut open = true;
        let mut connect = false;
        let mut cancel = false;
        egui::Window::new("Network endpoint")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Transport");
                    egui::ComboBox::from_id_salt("network-mode")
                        .selected_text(config.mode.label())
                        .show_ui(ui, |ui| {
                            for (mode, label) in [
                                (Mode::TcpClient, "TCP client"),
                                (Mode::TcpListener, "TCP listener"),
                                (Mode::UdpClient, "UDP client"),
                                (Mode::UdpListener, "UDP listener"),
                            ] {
                                ui.selectable_value(&mut config.mode, mode, label);
                            }
                        });
                });
                if matches!(config.mode, Mode::TcpClient | Mode::UdpClient) {
                    ui.label("Remote hostname / IP");
                    theme::control(ui, "Network host", egui::TextEdit::singleline(&mut config.host));
                    ui.horizontal(|ui| {
                        ui.label("Remote port");
                        theme::control(ui, "Network remote port", egui::DragValue::new(&mut config.port).range(1..=65535));
                    });
                }
                if config.mode != Mode::TcpClient {
                    ui.label("Local bind IP:port (IPv6: [::1]:5000)");
                    theme::control(ui, "Network bind", egui::TextEdit::singleline(&mut config.bind));
                    ui.small("Port 0 chooses a free port. Binding 0.0.0.0 or [::] accepts traffic on all interfaces.");
                }
                if config.mode == Mode::TcpClient {
                    ui.horizontal(|ui| {
                        ui.label("Connect timeout (ms)");
                        ui.add(egui::DragValue::new(&mut config.timeout_ms).range(1..=120000));
                    });
                }
                if config.mode.udp() {
                    ui.small("Each send, repeat iteration, and file chunk is one datagram. UDP listener replies use an explicit target selected in the terminal.");
                }
                if let Err(error) = config.validate() {
                    ui.colored_label(theme::ERROR, error);
                }
                ui.horizontal(|ui| {
                    let response = ui.add_enabled(config.validate().is_ok(), theme::primary_button("Open network"));
                    theme::trace_control(&response, "Open network");
                    if response.clicked() { connect = true; }
                    if theme::button(ui, "Cancel network").clicked() { cancel = true; }
                });
            });
        if connect {
            self.settings = SerialSettings {
                path: config.uri(),
                ..Default::default()
            };
            self.baud_control = BaudControl::new(self.settings.baud);
            self.connect();
        } else if open && !cancel {
            self.network_setup = Some(config);
        }
    }
}

impl Terminal {
    pub(super) fn packet_label(&self, index: usize) -> String {
        let metadata = if self.receive_mode == ReceiveMode::Line {
            self.lines.row(index).and_then(|row| row.network.as_ref())
        } else {
            self.history
                .get(index)
                .and_then(|event| event.network.as_ref())
        };
        metadata
            .map(|m| format!("UDP · {} → {}", m.source, m.destination))
            .unwrap_or_default()
    }
}

pub(super) fn bridge_framing_ui(
    ui: &mut egui::Ui,
    config: &mut signal_forge_network::BridgeConfig,
) {
    use signal_forge_network::{Ending, Framing};
    ui.label("Stream → UDP framing");
    let selected = match config.stream_to_udp {
        Framing::Line { .. } => "Line (LF / CRLF)",
        Framing::Fixed { .. } => "Fixed byte count",
        Framing::Idle { .. } => "Idle gap",
        Framing::Raw => "Raw chunks (advanced)",
    };
    let response = egui::ComboBox::from_id_salt("stream-udp-framing")
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for (policy, label) in [
                (Framing::default(), "Line (LF / CRLF) — default"),
                (Framing::Fixed { size: 64 }, "Fixed byte count"),
                (Framing::Idle { gap_ms: 20 }, "Idle gap"),
                (Framing::Raw, "Raw chunks (advanced)"),
            ] {
                ui.selectable_value(&mut config.stream_to_udp, policy, label);
            }
        })
        .response;
    help_ui::explain(response, "network.framing");
    match &mut config.stream_to_udp {
        Framing::Line { strip_ending } => {
            ui.checkbox(strip_ending, "Strip LF / CRLF from datagrams");
        }
        Framing::Fixed { size } => {
            ui.add(
                egui::DragValue::new(size)
                    .range(1..=65507)
                    .prefix("Bytes: "),
            );
        }
        Framing::Idle { gap_ms } => {
            ui.add(
                egui::DragValue::new(gap_ms)
                    .range(1..=60000)
                    .prefix("Idle ms: "),
            );
        }
        Framing::Raw => {
            ui.weak("One socket/serial read chunk per datagram; read boundaries are not message boundaries.");
        }
    }
    ui.label("UDP → stream: append after each datagram");
    let label = |ending: &Ending| match ending {
        Ending::None => "None",
        Ending::Lf => "LF",
        Ending::CrLf => "CRLF",
        Ending::Cr => "CR",
    };
    let response = egui::ComboBox::from_id_salt("udp-stream-ending")
        .selected_text(label(&config.udp_to_stream))
        .show_ui(ui, |ui| {
            for ending in [Ending::None, Ending::Lf, Ending::CrLf, Ending::Cr] {
                ui.selectable_value(&mut config.udp_to_stream, ending.clone(), label(&ending));
            }
        })
        .response;
    help_ui::explain(response, "network.framing");
    ui.weak("Incomplete lines/fixed packets are discarded when a bridge stops. Framing settings restore without starting a bridge.");
}
