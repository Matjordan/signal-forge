use super::*;
use signal_forge::{
    traffic_analysis::{Pattern, PatternMode},
    triggered_capture::{CaptureOptions, Trigger, TriggerState, TriggeredCapture},
};
pub(super) struct TriggerUi {
    path: String,
    dialog: Option<egui_file_dialog::FileDialog>,
    kind: u8,
    pattern: Pattern,
    idle_ms: u64,
    pre_bytes: usize,
    pre_ms: u64,
    post_bytes: usize,
    post_ms: u64,
    capture: Option<TriggeredCapture>,
}
impl Default for TriggerUi {
    fn default() -> Self {
        Self {
            path: String::new(),
            dialog: None,
            kind: 0,
            pattern: Pattern::default(),
            idle_ms: 1000,
            pre_bytes: 1024 * 1024,
            pre_ms: 5000,
            post_bytes: 65536,
            post_ms: 1000,
            capture: None,
        }
    }
}
impl TriggerUi {
    pub(super) fn stop(&self) {
        if let Some(capture) = &self.capture {
            capture.stop();
        }
    }
    pub(super) fn picker(&mut self, ctx: &egui::Context) {
        if let Some(dialog) = &mut self.dialog {
            dialog.update(ctx);
            if let Some(path) = dialog.take_picked() {
                if let Some(path) = path.to_str() {
                    self.path = path.into();
                }
            }
        }
    }
}
impl TerminalViewer<'_> {
    pub(super) fn trigger_panel(&mut self, ui: &mut egui::Ui, tab: &mut Terminal) {
        let capture = &mut tab.trigger;
        let active = capture.capture.as_ref().is_some_and(|capture| {
            matches!(
                capture.status().state,
                TriggerState::Armed | TriggerState::Capturing
            )
        });
        ui.label("One-shot rolling RX + TX capture");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut capture.path).desired_width(160.0));
            if ui.button("Save as…").clicked() {
                let mut dialog = egui_file_dialog::FileDialog::new()
                    .id(egui::Id::new((
                        "trigger-capture-picker",
                        tab.endpoint.id().clone(),
                    )))
                    .title("Choose new triggered capture file")
                    .default_file_name("triggered.jsonl");
                dialog.save_file();
                capture.dialog = Some(dialog);
            }
        });
        ui.add_enabled_ui(!active, |ui| {
            egui::ComboBox::from_id_salt("trigger-kind")
                .selected_text(match capture.kind {
                    0 => "Manual", 1 => "RX pattern", _ => "RX after idle",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut capture.kind, 0, "Manual");
                    ui.selectable_value(&mut capture.kind, 1, "RX pattern");
                    ui.selectable_value(&mut capture.kind, 2, "RX after idle");
                });
            if capture.kind == 1 {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("trigger-pattern")
                        .selected_text(format!("{:?}", capture.pattern.mode))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut capture.pattern.mode, PatternMode::Text, "Text");
                            ui.selectable_value(&mut capture.pattern.mode, PatternMode::Hex, "Hex");
                            ui.selectable_value(&mut capture.pattern.mode, PatternMode::Regex, "Byte regex");
                        });
                    ui.text_edit_singleline(&mut capture.pattern.value);
                });
            }
            if capture.kind == 2 {
                ui.add(egui::DragValue::new(&mut capture.idle_ms).range(1..=3600000).prefix("RX idle ").suffix(" ms"));
            }
            ui.horizontal_wrapped(|ui| {
                ui.label("Pre");
                ui.add(egui::DragValue::new(&mut capture.pre_bytes).range(0..=16*1024*1024).suffix(" B"));
                ui.add(egui::DragValue::new(&mut capture.pre_ms).range(0..=3600000).suffix(" ms"));
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Post");
                ui.add(egui::DragValue::new(&mut capture.post_bytes).range(0..=1024*1024*1024).suffix(" B"));
                ui.add(egui::DragValue::new(&mut capture.post_ms).range(0..=3600000).suffix(" ms"));
            });
            ui.weak("Pre bytes is a hard cap. Zero time disables that timer; zero post bytes disables its limit.");
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !active && tab.endpoint.state() == ConnectionState::Connected,
                    egui::Button::new("Arm capture"),
                )
                .clicked()
            {
                let options = CaptureOptions {
                    trigger: match capture.kind {
                        0 => Trigger::Manual,
                        1 => Trigger::Pattern(capture.pattern.clone()),
                        _ => Trigger::RxIdle(Duration::from_millis(capture.idle_ms)),
                    },
                    pre_bytes: capture.pre_bytes,
                    pre_duration: Duration::from_millis(capture.pre_ms),
                    post_bytes: capture.post_bytes,
                    post_duration: Duration::from_millis(capture.post_ms),
                };
                match TriggeredCapture::start(
                    std::path::Path::new(&capture.path),
                    tab.endpoint.id().clone(),
                    self.bus,
                    options,
                ) {
                    Ok(worker) => {
                        capture.capture = Some(worker);
                        tab.error = None;
                    }
                    Err(error) => tab.error = Some(error),
                }
            }
            if ui
                .add_enabled(active, egui::Button::new("Trigger now"))
                .clicked()
            {
                if let Some(worker) = &capture.capture {
                    worker.trigger();
                }
            }
            if ui
                .add_enabled(active, egui::Button::new("Stop capture"))
                .clicked()
            {
                capture.stop();
            }
        });
        if let Some(worker) = &capture.capture {
            let status = worker.status();
            ui.label(format!(
                "{:?} · buffered {} B / {} events · saved {} B / {} events · {} dropped",
                status.state,
                status.buffered_bytes,
                status.buffered_events,
                status.bytes,
                status.events,
                status.dropped
            ));
        }
    }
}
