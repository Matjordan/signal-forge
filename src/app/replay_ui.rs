use super::*;
use signal_forge::replay::{ReplayConfig, ReplayFormat, ReplayState, ReplayStream};
pub(super) struct ReplaySetup {
    config: ReplayConfig,
    picker: Option<egui_file_dialog::FileDialog>,
}
impl Default for ReplaySetup {
    fn default() -> Self {
        Self {
            config: ReplayConfig::default(),
            picker: None,
        }
    }
}
impl Workbench {
    pub(super) fn replay_setup_ui(&mut self, ctx: &egui::Context) {
        let Some(mut setup) = self.replay_setup.take() else {
            return;
        };
        let mut open = true;
        let mut accept = false;
        let mut cancel = false;
        if let Some(dialog) = &mut setup.picker {
            dialog.update(ctx);
            if let Some(path) = dialog.take_picked() {
                if let Some(path) = path.to_str() {
                    setup.config.path = path.into();
                } else {
                    self.error = Some("Replay path must be valid UTF-8".into());
                }
            }
        }
        egui::Window::new("Open replay source")
            .id(egui::Id::new("open-replay-source"))
            .open(&mut open)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label("Read-only playback; starts paused and never modifies the source.");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut setup.config.path);
                    if ui.button("Browse…").clicked() {
                        let mut dialog = egui_file_dialog::FileDialog::new()
                            .id(egui::Id::new("replay-source-picker"))
                            .title("Choose recording or capture");
                        dialog.pick_file();
                        setup.picker = Some(dialog);
                    }
                });
                egui::ComboBox::from_id_salt("replay-format")
                    .selected_text(format!("{:?}", setup.config.format))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut setup.config.format,
                            ReplayFormat::Raw,
                            "Raw binary / rendered ASCII file",
                        );
                        ui.selectable_value(
                            &mut setup.config.format,
                            ReplayFormat::Hex,
                            "Hex RX recording",
                        );
                        ui.selectable_value(
                            &mut setup.config.format,
                            ReplayFormat::Capture,
                            "Signal Forge JSONL capture",
                        );
                    });
                egui::ComboBox::from_id_salt("replay-stream")
                    .selected_text(format!("{:?}", setup.config.stream))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut setup.config.stream,
                            ReplayStream::Rx,
                            "RX / bridge A → B",
                        );
                        ui.selectable_value(
                            &mut setup.config.stream,
                            ReplayStream::Tx,
                            "TX / bridge B → A",
                        );
                        ui.selectable_value(
                            &mut setup.config.stream,
                            ReplayStream::Both,
                            "Both streams (merged file order)",
                        );
                    });
                if setup.config.format != ReplayFormat::Capture {
                    ui.horizontal(|ui| {
                        ui.label("Raw/Hex chunk");
                        ui.add(
                            egui::DragValue::new(&mut setup.config.chunk)
                                .range(1..=65536)
                                .suffix(" B"),
                        );
                        ui.label("Pacing");
                        ui.add(
                            egui::DragValue::new(&mut setup.config.delay_ms)
                                .range(0..=60000)
                                .suffix(" ms"),
                        );
                    });
                    ui.weak(
                        "ASCII recording escapes are rendered text, not an exact binary replay.",
                    );
                }
                accept = ui.button("Open paused replay").clicked();
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        if accept {
            self.settings = SerialSettings {
                path: setup.config.uri(),
                ..Default::default()
            };
            self.baud_control = BaudControl::new(self.settings.baud);
            self.connect();
        } else if open && !cancel && !ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.replay_setup = Some(setup);
        }
    }
}
impl TerminalViewer<'_> {
    pub(super) fn playback_panel(ui: &mut egui::Ui, tab: &mut Terminal) {
        let Some(controller) = tab.endpoint.playback() else {
            return;
        };
        let status = controller.status();
        ui.label(format!(
            "Read-only replay · {:?} · {} / {} events · {} / {} bytes",
            status.state, status.events, status.total_events, status.bytes, status.total_bytes
        ));
        if let Some(warning) = &status.warning {
            ui.colored_label(theme::WARNING, warning);
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    status.state == ReplayState::Paused,
                    egui::Button::new("Play"),
                )
                .clicked()
            {
                controller.play();
            }
            if ui
                .add_enabled(
                    status.state == ReplayState::Playing,
                    egui::Button::new("Pause playback"),
                )
                .clicked()
            {
                controller.pause();
            }
            if ui
                .add_enabled(
                    status.state == ReplayState::Paused,
                    egui::Button::new("Step"),
                )
                .clicked()
            {
                controller.step();
            }
            if ui
                .add_enabled(
                    matches!(
                        status.state,
                        ReplayState::Paused | ReplayState::Playing | ReplayState::Completed
                    ),
                    egui::Button::new("Stop / Reset"),
                )
                .clicked()
            {
                controller.reset();
            }
            let mut fast = status.fast;
            if ui.checkbox(&mut fast, "Maximum speed").changed() {
                controller.set_fast(fast);
            }
        });
        ui.weak(
            "Original timing by default. Reset stops attached bridges; restart output explicitly.",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replay_uses_normal_terminal_views_analysis_and_playback_controls() {
        use signal_forge::{
            replay::ReplayEndpoint,
            traffic_analysis::{Pattern, PatternMode},
        };
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recording");
        std::fs::write(&path, b"READY\r\nERROR\0\xff\r\n").unwrap();
        let config = ReplayConfig {
            path: path.to_str().unwrap().into(),
            ..Default::default()
        };
        let bus = TrafficBus::default();
        let subscription = bus.subscribe_tracked(8);
        let endpoint = ReplayEndpoint::open(config.clone(), bus).unwrap();
        let control = endpoint.controller();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while control.status().state == ReplayState::Loading {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        let mut tab = Terminal::new(
            endpoint,
            SerialSettings {
                path: config.uri(),
                ..Default::default()
            },
        );
        assert!(tab.endpoint.read_only());
        assert!(subscription.receiver.try_recv().is_err());
        control.step();
        tab.receive(
            subscription
                .receiver
                .recv_timeout(Duration::from_secs(2))
                .unwrap(),
        );
        assert_eq!(tab.rx_bytes, 16);
        assert_eq!(tab.history[0].bytes.as_ref(), b"READY\r\nERROR\0\xff\r\n");
        tab.analysis.search = Pattern {
            mode: PatternMode::Hex,
            value: "00 FF".into(),
        };
        tab.refresh_view();
        assert_eq!(tab.view.matches.len(), 1);
        for mode in [ReceiveMode::Line, ReceiveMode::RawChunks, ReceiveMode::Hex] {
            tab.receive_mode = mode;
            tab.view.dirty = true;
            tab.refresh_view();
            assert!(!tab.view.rows.is_empty());
            assert_eq!(tab.view.matches.len(), 1);
        }
        let context = egui::Context::default();
        let output = context.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default()
                .show(ctx, |ui| TerminalViewer::playback_panel(ui, &mut tab));
        });
        fn text(shape: &egui::epaint::Shape, output: &mut String) {
            match shape {
                egui::epaint::Shape::Text(value) => output.push_str(value.galley.text()),
                egui::epaint::Shape::Vec(values) => {
                    for value in values {
                        text(value, output);
                    }
                }
                _ => {}
            }
        }
        let mut labels = String::new();
        for shape in output.shapes {
            text(&shape.shape, &mut labels);
        }
        for expected in [
            "Read-only replay",
            "Play",
            "Pause playback",
            "Step",
            "Stop / Reset",
            "Maximum speed",
        ] {
            assert!(labels.contains(expected), "{labels}");
        }
        assert_eq!(std::fs::read(path).unwrap(), b"READY\r\nERROR\0\xff\r\n");
    }
}
