use super::{Workbench, ACCENT};
use eframe::egui::{self, RichText};
use signal_forge::{
    endpoint::{ConnectionState, EndpointId},
    presets::{Preset, PresetLibrary, PresetTarget, Profile, RepeatSettings},
    send::{Encoding, LineEnding},
};
use std::path::Path;

impl Workbench {
    fn commit_library(&mut self, next: PresetLibrary) {
        if !self.library_ok {
            self.error = Some(
                "Invalid preset file was preserved. Repair it and restart before saving.".into(),
            );
            return;
        }
        match next.save() {
            Ok(()) => {
                self.library = next;
                self.error = None;
            }
            Err(error) => self.error = Some(format!("Presets: {error}")),
        }
    }
    fn dispatch_preset(&mut self, preset: Preset) {
        let id = match &preset.target {
            PresetTarget::Selected => self.selected.clone(),
            PresetTarget::Endpoint(id) => Some(EndpointId(id.clone())),
        };
        let Some(id) = id else {
            self.error = Some("Select a preset target terminal first".into());
            return;
        };
        let bytes = match preset.bytes() {
            Ok(bytes) => bytes,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        for (_, tab) in self.dock.iter_all_tabs_mut() {
            if tab.endpoint.id() != &id {
                continue;
            }
            let result = if let Some(spec) = preset.repeat_spec() {
                if tab.repeat.as_ref().is_some_and(|handle| handle.is_active()) {
                    Err("Stop the current repeat before starting a repeating preset".to_owned())
                } else {
                    tab.endpoint
                        .start_repeat(bytes, spec)
                        .map(|handle| {
                            tab.repeat = Some(handle);
                            tab.repeat_interval_ms = spec.interval.as_millis() as u64;
                            tab.continuous = spec.count.is_none();
                            tab.repeat_count = spec.count.unwrap_or(10);
                        })
                        .map_err(|e| e.to_string())
                }
            } else {
                tab.endpoint.send(bytes).map_err(|e| e.to_string())
            };
            if result.is_ok() {
                tab.input = preset.payload.clone();
                tab.encoding = preset.encoding;
                tab.escapes = preset.escapes;
                tab.ending = preset.ending;
            }
            self.error = result
                .err()
                .map(|error| format!("{} — {}: {error}", preset.name, id.0));
            return;
        }
        self.error = Some(format!(
            "{}: target {} has no open terminal",
            preset.name, id.0
        ));
    }
    pub(super) fn presets_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Preset messages");
        egui::ComboBox::from_id_salt("preset_target")
            .selected_text(
                self.selected
                    .as_ref()
                    .map(|id| id.0.as_str())
                    .unwrap_or("Select target"),
            )
            .show_ui(ui, |ui| {
                for (_, tab) in self.dock.iter_all_tabs() {
                    if tab.endpoint.state() == ConnectionState::Connected {
                        ui.selectable_value(
                            &mut self.selected,
                            Some(tab.endpoint.id().clone()),
                            tab.endpoint.display_name(),
                        );
                    }
                }
            });
        ui.small(
            "Selected-target presets use this terminal. Clicking inside a pane also selects it.",
        );
        egui::ComboBox::from_id_salt("preset_profile")
            .selected_text(&self.library.profiles[self.profile_index].name)
            .show_ui(ui, |ui| {
                for (index, profile) in self.library.profiles.iter().enumerate() {
                    ui.selectable_value(&mut self.profile_index, index, &profile.name);
                }
            });
        let presets = self.library.profiles[self.profile_index].presets.clone();
        if presets.is_empty() {
            ui.label("No presets in this profile");
        }
        for (index, preset) in presets.iter().enumerate() {
            let mut action = None;
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(RichText::new(&preset.name).color(ACCENT))
                    .on_hover_text(format!("{}\n{}", preset.description, preset.shortcut))
                    .clicked()
                {
                    self.dispatch_preset(preset.clone());
                }
                ui.add_enabled_ui(self.library_ok, |ui| {
                    if ui.small_button("Edit").clicked() {
                        action = Some("edit");
                    }
                    if ui
                        .add_enabled(index > 0, egui::Button::new("Up").small())
                        .clicked()
                    {
                        action = Some("up");
                    }
                    if ui
                        .add_enabled(index + 1 < presets.len(), egui::Button::new("Down").small())
                        .clicked()
                    {
                        action = Some("down");
                    }
                    if ui.small_button("Delete").clicked() {
                        action = Some("delete");
                    }
                });
            });
            match action {
                Some("edit") => {
                    self.preset_draft = preset.clone();
                    self.editor_profile = self.profile_index;
                    self.editor_index = Some(index);
                    self.preset_editor_open = true;
                }
                Some("up" | "down" | "delete") => {
                    let mut next = self.library.clone();
                    let list = &mut next.profiles[self.profile_index].presets;
                    match action {
                        Some("up") => list.swap(index, index - 1),
                        Some("down") => list.swap(index, index + 1),
                        _ => {
                            list.remove(index);
                        }
                    }
                    self.commit_library(next);
                    break;
                }
                _ => {}
            }
        }
        if ui
            .add_enabled(self.library_ok, egui::Button::new("New preset"))
            .clicked()
        {
            self.preset_draft = Preset::default();
            self.editor_profile = self.profile_index;
            self.editor_index = None;
            self.preset_editor_open = true;
        }
        ui.separator();
        ui.add_enabled_ui(self.library_ok, |ui| {
            ui.label("New profile name");
            ui.text_edit_singleline(&mut self.new_profile);
            if ui.button("Create profile").clicked() {
                let name = self.new_profile.trim().to_owned();
                let mut next = self.library.clone();
                next.profiles.push(Profile {
                    name,
                    presets: Vec::new(),
                });
                let count = next.profiles.len();
                self.commit_library(next);
                if self.library.profiles.len() == count {
                    self.profile_index = count - 1;
                    self.new_profile.clear();
                }
            }
            ui.label("Profile JSON file");
            ui.text_edit_singleline(&mut self.profile_file);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !self.profile_file.is_empty(),
                        egui::Button::new("Export profile"),
                    )
                    .clicked()
                {
                    self.error = self.library.profiles[self.profile_index]
                        .export(Path::new(&self.profile_file))
                        .err();
                }
                if ui
                    .add_enabled(
                        !self.profile_file.is_empty(),
                        egui::Button::new("Import profile"),
                    )
                    .clicked()
                {
                    match Profile::import(Path::new(&self.profile_file)) {
                        Ok(profile) => {
                            let mut next = self.library.clone();
                            let index = next
                                .profiles
                                .iter()
                                .position(|p| p.name == profile.name)
                                .unwrap_or(next.profiles.len());
                            if index == next.profiles.len() {
                                next.profiles.push(profile);
                            } else {
                                next.profiles[index] = profile;
                            }
                            self.commit_library(next);
                            if self.error.is_none() {
                                self.profile_index = index;
                                self.preset_editor_open = false;
                            }
                        }
                        Err(error) => self.error = Some(format!("Import failed: {error}")),
                    }
                }
            });
        });
        ui.small(
            "Import replaces a profile with the same name. Loading/importing never transmits.",
        );
        if !self.library_ok {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                "Preset file is invalid and preserved; repair it and restart.",
            );
        }
    }
    pub(super) fn preset_editor(&mut self, ctx: &egui::Context) {
        if !self.preset_editor_open {
            return;
        }
        let mut open = true;
        let mut saved = false;
        egui::Window::new("Preset editor")
            .open(&mut open)
            .default_width(440.0)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Profile: {}",
                    self.library.profiles[self.editor_profile].name
                ));
                ui.label("Name");
                ui.text_edit_singleline(&mut self.preset_draft.name);
                ui.label("Payload");
                ui.text_edit_multiline(&mut self.preset_draft.payload);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.preset_draft.encoding, Encoding::Text, "Text");
                    ui.selectable_value(
                        &mut self.preset_draft.encoding,
                        Encoding::Hex,
                        "Hex bytes",
                    );
                    ui.checkbox(&mut self.preset_draft.escapes, "Interpret escapes");
                });
                egui::ComboBox::from_id_salt("preset_ending")
                    .selected_text(format!("Ending: {:?}", self.preset_draft.ending))
                    .show_ui(ui, |ui| {
                        for value in [
                            LineEnding::None,
                            LineEnding::Cr,
                            LineEnding::Lf,
                            LineEnding::CrLf,
                        ] {
                            ui.selectable_value(
                                &mut self.preset_draft.ending,
                                value,
                                format!("{value:?}"),
                            );
                        }
                    });
                let mut fixed = matches!(self.preset_draft.target, PresetTarget::Endpoint(_));
                if ui.checkbox(&mut fixed, "Fixed endpoint target").changed() {
                    self.preset_draft.target = if fixed {
                        PresetTarget::Endpoint(
                            self.selected
                                .as_ref()
                                .map(|id| id.0.clone())
                                .unwrap_or_default(),
                        )
                    } else {
                        PresetTarget::Selected
                    };
                }
                if let PresetTarget::Endpoint(id) = &mut self.preset_draft.target {
                    ui.text_edit_singleline(id);
                    ui.small("Stable endpoint ID, e.g. serial:/dev/ttyUSB0");
                }
                ui.label("Description");
                ui.text_edit_singleline(&mut self.preset_draft.description);
                ui.label("Shortcut (Ctrl+1 ... Ctrl+9, or empty)");
                ui.text_edit_singleline(&mut self.preset_draft.shortcut);
                let mut repeating = self.preset_draft.repeat.is_some();
                if ui.checkbox(&mut repeating, "Repeat this preset").changed() {
                    self.preset_draft.repeat = if repeating {
                        Some(RepeatSettings {
                            interval_ms: 1000,
                            count: Some(10),
                        })
                    } else {
                        None
                    };
                }
                if let Some(repeat) = &mut self.preset_draft.repeat {
                    ui.add(
                        egui::DragValue::new(&mut repeat.interval_ms)
                            .range(1..=86400000)
                            .suffix(" ms"),
                    );
                    let mut continuous = repeat.count.is_none();
                    if ui.checkbox(&mut continuous, "Until stopped").changed() {
                        repeat.count = if continuous { None } else { Some(10) };
                    }
                    if let Some(count) = &mut repeat.count {
                        ui.add(
                            egui::DragValue::new(count)
                                .range(1..=1000000)
                                .suffix(" sends"),
                        );
                    }
                }
                if let Err(error) = self.preset_draft.validate() {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                if ui
                    .add_enabled(
                        self.preset_draft.validate().is_ok(),
                        egui::Button::new("Save preset"),
                    )
                    .clicked()
                {
                    let mut next = self.library.clone();
                    let list = &mut next.profiles[self.editor_profile].presets;
                    if let Some(index) = self.editor_index {
                        list[index] = self.preset_draft.clone();
                    } else {
                        list.push(self.preset_draft.clone());
                    }
                    self.commit_library(next);
                    saved = self.error.is_none();
                }
            });
        self.preset_editor_open = open && !saved;
    }
    pub(super) fn preset_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() || self.preset_editor_open {
            return;
        }
        let keys = [
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
            egui::Key::Num6,
            egui::Key::Num7,
            egui::Key::Num8,
            egui::Key::Num9,
        ];
        for (index, key) in keys.into_iter().enumerate() {
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, key)) {
                let shortcut = format!("Ctrl+{}", index + 1);
                if let Some(preset) = self.library.profiles[self.profile_index]
                    .presets
                    .iter()
                    .find(|p| p.shortcut == shortcut)
                    .cloned()
                {
                    self.dispatch_preset(preset);
                }
            }
        }
    }
}
