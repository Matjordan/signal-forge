use super::*;
use signal_forge::session::{ArtifactKind, Session};
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Default)]
pub(super) struct SessionUi {
    pub open: bool,
    create: bool,
    name: String,
    folder: String,
    picker: Option<egui_file_dialog::FileDialog>,
    artifact_picker: Option<egui_file_dialog::FileDialog>,
    edited: Option<Instant>,
    pub(super) error: Option<String>,
    opener: Option<std::process::Child>,
}
impl Workbench {
    pub(super) fn save_session_context(&mut self, closed: bool) -> Result<(), String> {
        if let Some(session) = &mut self.session {
            session.save_context(&self.config, closed)?;
            self.session_ui.edited = None;
            self.session_ui.error = None;
        }
        Ok(())
    }
    pub(super) fn collect_session_artifacts(&mut self) {
        for view in &mut self.bridges {
            if let Some(path) = view.pending_artifact.take() {
                self.pending_artifacts
                    .push((path, ArtifactKind::BridgeCapture));
            }
        }
        let artifacts = std::mem::take(&mut self.pending_artifacts);
        if let Some(session) = &mut self.session {
            for (path, kind) in artifacts {
                if let Err(error) = session.register(&path, kind) {
                    self.session_ui.open = true;
                    self.session_ui.error = Some(format!(
                        "Artifact index: {error}. Use Add artifact to retry."
                    ));
                }
            }
        }
    }
    pub(super) fn finish_session_outputs(&mut self) {
        self.collect_session_artifacts();
        for (_, tab) in self.dock.iter_all_tabs_mut() {
            if let Some(recording) = &mut tab.recording {
                recording.finish();
            }
            tab.trigger.finish();
        }
        for view in &mut self.bridges {
            view.finish_capture();
        }
    }
    fn session_defaults(&mut self) {
        if let Some(session) = &self.session {
            for (index, (_, tab)) in self.dock.iter_all_tabs_mut().enumerate() {
                tab.recording_path = session
                    .suggested_path(&format!("rx-{}", index + 1), "bin")
                    .display()
                    .to_string();
                tab.trigger
                    .set_path(session.suggested_path(&format!("trigger-{}", index + 1), "jsonl"));
            }
            for (index, view) in self.bridges.iter_mut().enumerate() {
                view.set_capture_path(
                    session.suggested_path(&format!("bridge-{}", index + 1), "jsonl"),
                );
            }
        }
    }
    fn start_session(&mut self, create: bool, folder: &Path, name: &str) -> Result<(), String> {
        if self.session.is_some() {
            return Err("Close the active session first".into());
        }
        self.snapshot_workspace();
        if create {
            let session = Session::create(folder, name, &self.config)?;
            self.finish_session_outputs();
            self.session = Some(session);
        } else {
            let (session, config) = Session::open(folder)?;
            self.finish_session_outputs();
            self.bridges.clear();
            for (_, tab) in self.dock.iter_all_tabs_mut() {
                tab.stop_repeat();
                tab.endpoint.disconnect();
            }
            self.pairs.clear();
            self.dock = DockState::new(Vec::new());
            self.selected = None;
            self.config = config;
            self.restore_workspace();
            self.config_recoverable = true;
            self.session = Some(session);
        }
        self.session_ui.edited = None;
        self.session_defaults();
        log::info!(
            "Session opened: {}",
            self.session.as_ref().unwrap().root.display()
        );
        Ok(())
    }
    fn close_session(&mut self) -> Result<(), String> {
        self.finish_session_outputs();
        self.snapshot_workspace();
        self.save_session_context(true)?;
        if let Some(session) = self.session.take() {
            for (_, tab) in self.dock.iter_all_tabs_mut() {
                if Path::new(&tab.recording_path).starts_with(&session.root) {
                    tab.recording_path.clear();
                }
                tab.trigger.clear_session_path(&session.root);
            }
            for view in &mut self.bridges {
                view.clear_session_path(&session.root);
            }
            log::info!("Session closed: {}", session.root.display());
        }
        Ok(())
    }
    pub(super) fn session_panel(&mut self, ctx: &egui::Context) {
        if self.setup.is_none()
            && self.replay_setup.is_none()
            && ctx.input_mut(|input| {
                input.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::E)
            })
        {
            self.session_ui.open = !self.session_ui.open;
        }

        if let Some(process) = &mut self.session_ui.opener {
            match process.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        self.session_ui.error = Some(format!(
                            "Desktop opener failed ({status}); use Copy path to locate the file."
                        ));
                    }
                    self.session_ui.opener = None;
                }
                Err(error) => {
                    self.session_ui.error = Some(error.to_string());
                    self.session_ui.opener = None;
                }
                Ok(None) => {}
            }
        }
        if let Some(dialog) = &mut self.session_ui.picker {
            dialog.update(ctx);
            if let Some(path) = dialog.take_picked() {
                match path.to_str() {
                    Some(value) => self.session_ui.folder = value.into(),
                    None => self.session_ui.error = Some("Session path must be UTF-8".into()),
                }
            }
        }
        if let Some(dialog) = &mut self.session_ui.artifact_picker {
            dialog.update(ctx);
            if let Some(path) = dialog.take_picked() {
                self.pending_artifacts
                    .push((path, ArtifactKind::Associated));
            }
        }
        let mut open = self.session_ui.open;
        let mut action = 0;
        let mut locate = None;
        let content = |ui: &mut egui::Ui| {
            if let Some(session) = &mut self.session {
                ui.heading(&session.metadata.name);
                ui.label(session.root.display().to_string());
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Save session").clicked() {
                        action = 3;
                    }
                    if ui.button("Open session folder").clicked() {
                        locate = Some(session.root.clone());
                    }
                    if ui.button("Copy folder path").clicked() {
                        ui.ctx().copy_text(session.root.display().to_string());
                    }
                    if theme::button(ui, "Close session").clicked() {
                        action = 4;
                    }
                    if ui.button("Copy notes").clicked() {
                        ui.ctx().copy_text(session.notes.clone());
                    }
                });
                ui.weak("Close finalizes recordings/captures; live connections remain open.");
                ui.separator();
                ui.label(if self.session_ui.edited.is_some() {
                    "Notes · unsaved"
                } else {
                    "Notes · saved"
                });
                egui::ScrollArea::vertical()
                    .id_salt("session-notes-scroll")
                    .max_height(160.0)
                    .show(ui, |ui| {
                        if theme::control(
                            ui,
                            "Session notes",
                            egui::TextEdit::multiline(&mut session.notes)
                                .id(egui::Id::new("session-notes"))
                                .desired_width(f32::INFINITY)
                                .desired_rows(6)
                                .char_limit(1024 * 1024),
                        )
                        .changed()
                        {
                            self.session_ui.edited = Some(Instant::now());
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(format!("Artifacts ({})", session.metadata.artifacts.len()));
                    if ui.button("Add artifact…").clicked() {
                        let mut dialog = egui_file_dialog::FileDialog::new()
                            .id(egui::Id::new("session-artifact-picker"))
                            .title("Associate an existing artifact");
                        dialog.pick_file();
                        self.session_ui.artifact_picker = Some(dialog);
                    }
                });
                egui::ScrollArea::vertical()
                    .id_salt("session-artifacts-scroll")
                    .max_height(250.0)
                    .show(ui, |ui| {
                        for (index, artifact) in session.metadata.artifacts.iter().enumerate() {
                            ui.push_id(index, |ui| {
                                let path = session.artifact_path(artifact);
                                let exists = path.is_file();
                                ui.label(format!(
                                    "{:?} · {}{}",
                                    artifact.kind,
                                    artifact.path,
                                    if exists { "" } else { " · missing/moved" }
                                ));
                                ui.horizontal(|ui| {
                                    if ui
                                        .add_enabled(exists, egui::Button::new("Open file"))
                                        .clicked()
                                    {
                                        locate = Some(path.clone());
                                    }
                                    if ui
                                        .add_enabled(
                                            path.parent().is_some_and(Path::is_dir),
                                            egui::Button::new("Open containing folder"),
                                        )
                                        .clicked()
                                    {
                                        locate = path.parent().map(Path::to_path_buf);
                                    }
                                    if ui.button("Copy path").clicked() {
                                        ui.ctx().copy_text(path.display().to_string());
                                    }
                                });
                            });
                        }
                    });
            } else {
                ui.horizontal(|ui| {
                    theme::tab_value(
                        ui,
                        &mut self.session_ui.create,
                        true,
                        "New session",
                        "New session",
                    );
                    theme::tab_value(
                        ui,
                        &mut self.session_ui.create,
                        false,
                        "Open session",
                        "Open session",
                    );
                });
                if self.session_ui.create {
                    ui.label("Session name");
                    theme::control(
                        ui,
                        "Session name",
                        egui::TextEdit::singleline(&mut self.session_ui.name),
                    );
                    ui.weak("Choose a new or empty folder. Enter a new folder path to create it.");
                } else {
                    ui.weak("Choose a folder containing session.json; its workspace restores disconnected.");
                }
                ui.horizontal(|ui| {
                    theme::control(
                        ui,
                        "Session folder",
                        egui::TextEdit::singleline(&mut self.session_ui.folder),
                    );
                    if ui.button("Choose folder…").clicked() {
                        let mut dialog = egui_file_dialog::FileDialog::new()
                            .id(egui::Id::new("session-folder-picker"))
                            .title("Choose session folder");
                        dialog.pick_directory();
                        self.session_ui.picker = Some(dialog);
                    }
                });
                if theme::button(
                    ui,
                    if self.session_ui.create {
                        "Create session"
                    } else {
                        "Open session folder"
                    },
                )
                .clicked()
                {
                    action = if self.session_ui.create { 1 } else { 2 };
                }
            }
            if let Some(error) = &self.session_ui.error {
                ui.colored_label(theme::ERROR, error);
                if self.session.is_some() {
                    ui.weak("Reload adopts edits on disk and discards unsaved notes; terminals stay unchanged. Copy notes first if needed.");
                    if ui.button("Reload session files from disk").clicked() {
                        action = 5;
                    }
                }
            }
        };
        egui::Window::new("Debug session")
            .id(egui::Id::new("debug-session"))
            .open(&mut open)
            .default_width(650.0)
            .default_height(550.0)
            .show(ctx, content);

        self.session_ui.open = open;
        let result = match action {
            1 | 2 => {
                let folder = PathBuf::from(&self.session_ui.folder);
                let name = self.session_ui.name.clone();
                self.start_session(action == 1, &folder, &name)
            }
            3 => {
                self.snapshot_workspace();
                self.save_session_context(false)
            }
            4 => self.close_session(),
            5 => {
                let root = self.session.as_ref().unwrap().root.clone();
                Session::open(&root).map(|(session, _)| {
                    self.session = Some(session);
                    self.session_ui.edited = None;
                })
            }
            _ => Ok(()),
        };
        if action != 0 {
            self.session_ui.error = result.err();
        }
        if let Some(path) = locate {
            if self.session_ui.opener.is_none() {
                match std::process::Command::new("xdg-open")
                    .arg(&path)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                {
                    Ok(process) => self.session_ui.opener = Some(process),
                    Err(error) => {
                        self.session_ui.error = Some(format!(
                            "Cannot open {}: {error}. Use Copy path.",
                            path.display()
                        ))
                    }
                }
            }
        }
        if self
            .session_ui
            .edited
            .is_some_and(|edited| edited.elapsed() >= Duration::from_secs(1))
        {
            if let Some(session) = &mut self.session {
                match session.save_notes() {
                    Ok(()) => self.session_ui.edited = None,
                    Err(error) => {
                        self.session_ui.error = Some(error);
                        self.session_ui.edited = Some(Instant::now());
                    }
                }
            }
        }
    }
}
