use super::*;
use signal_forge::updater::{self, InstalledUpdate, Offer, PreparedUpdate, Progress};
use std::sync::{mpsc, Mutex};
pub(super) type RestartRequest = Arc<Mutex<Option<InstalledUpdate>>>;

pub(super) enum UpdateState {
    Checking,
    Quiet,
    Available(Offer),
    Deferred,
    Working(Progress),
    Failed(String),
    Restarting,
}
enum Event {
    Checked(updater::Result<Option<Offer>>),
    Progress(Progress),
    Prepared(updater::Result<PreparedUpdate>),
}
pub(super) struct UpdateController {
    pub state: UpdateState,
    sender: mpsc::Sender<Event>,
    receiver: mpsc::Receiver<Event>,
    restart: RestartRequest,
    prepared: Option<PreparedUpdate>,
}
impl UpdateController {
    pub fn new(enabled: bool, restart: RestartRequest) -> Self {
        let (sender, receiver) = mpsc::channel();
        let controller = Self {
            state: if enabled {
                UpdateState::Checking
            } else {
                UpdateState::Quiet
            },
            sender,
            receiver,
            restart,
            prepared: None,
        };
        if enabled {
            let tx = controller.sender.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Event::Checked(updater::check()));
            });
        }
        controller
    }
    /// The only entry point into artifact downloading requires an explicit approval action.
    fn approve(&mut self) {
        let UpdateState::Available(offer) = &self.state else {
            return;
        };
        let offer = offer.clone();
        let tx = self.sender.clone();
        self.state = UpdateState::Working(Progress::Downloading {
            received: 0,
            total: None,
        });
        std::thread::spawn(move || {
            let result = std::env::current_exe()
                .map_err(|e| e.to_string())
                .and_then(|target| {
                    updater::prepare(&offer, &target, |state| {
                        let _ = tx.send(Event::Progress(state));
                    })
                });
            let _ = tx.send(Event::Prepared(result));
        });
    }
    fn defer(&mut self) {
        if matches!(self.state, UpdateState::Available(_)) {
            self.state = UpdateState::Deferred;
        }
    }
    fn poll(&mut self) {
        // Let Installing paint for one frame before the short atomic replacement.
        if let Some(prepared) = self.prepared.take() {
            match prepared.install() {
                Ok(installed) => {
                    *self.restart.lock().unwrap() = Some(installed);
                    self.state = UpdateState::Restarting;
                }
                Err(error) => self.state = UpdateState::Failed(error),
            }
            return;
        }
        // Bound per-frame handling even when the worker reports many download chunks.
        for _ in 0..64 {
            let Ok(event) = self.receiver.try_recv() else {
                break;
            };
            match event {
                Event::Checked(Ok(Some(offer))) if matches!(self.state, UpdateState::Checking) => {
                    self.state = UpdateState::Available(offer)
                }
                Event::Checked(Ok(None)) => self.state = UpdateState::Quiet,
                Event::Checked(Err(error)) => {
                    log::info!("Update check unavailable: {error}");
                    self.state = UpdateState::Quiet;
                }
                Event::Checked(_) => {}
                Event::Progress(progress) if matches!(self.state, UpdateState::Working(_)) => {
                    self.state = UpdateState::Working(progress)
                }
                Event::Progress(_) => {}
                Event::Prepared(Ok(prepared)) => {
                    self.state = UpdateState::Working(Progress::Installing);
                    self.prepared = Some(prepared);
                    return;
                }
                Event::Prepared(Err(error)) => self.state = UpdateState::Failed(error),
            }
        }
    }
}
impl Workbench {
    pub(super) fn update_prompt(&mut self, ctx: &egui::Context) {
        self.updater.poll();
        if matches!(self.updater.state, UpdateState::Restarting) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if matches!(
            self.updater.state,
            UpdateState::Checking | UpdateState::Quiet | UpdateState::Deferred
        ) {
            return;
        }
        let mut approve = false;
        let mut dismiss = false;
        egui::Modal::new(egui::Id::new("application-update")).show(ctx, |ui| {
            ui.set_width(400.0);
            ui.heading("Signal Forge update"); ui.separator();
            match &self.updater.state {
                UpdateState::Available(offer) => {
                    ui.label(format!("Installed: {}", env!("CARGO_PKG_VERSION")));
                    ui.label(format!("Available: {}", offer.version));
                    ui.hyperlink_to("Release notes", &offer.notes_url);
                    ui.label("Install the verified official release and restart. Your workspace and presets are preserved; terminals return disconnected.");
                    ui.horizontal(|ui| {
                        approve = ui.add(theme::primary_button("Update Now")).clicked();
                        dismiss = ui.button("Not Now").clicked();
                    });
                }
                UpdateState::Working(progress) => {
                    ui.spinner();
                    match progress {
                        Progress::Downloading { received, total } => {
                            ui.label("Downloading update");
                            if let Some(total) = total.filter(|n| *n > 0) { ui.add(egui::ProgressBar::new(*received as f32 / total as f32).show_percentage()); }
                            ui.weak(format!("{:.1} MiB downloaded", *received as f64 / 1048576.0));
                        }
                        Progress::Verifying => { ui.label("Verifying checksum, platform and version"); }
                        Progress::Installing => { ui.label("Installing update"); }
                        Progress::Restarting => { ui.label("Restarting Signal Forge"); }
                    }
                    ui.weak("Closing before installation keeps the current executable.");
                }
                UpdateState::Failed(error) => {
                    ui.colored_label(theme::ERROR, "Update failed"); ui.label(error);
                    ui.label("Your existing application is still available.");
                    dismiss = ui.button("Continue with current version").clicked();
                }
                UpdateState::Restarting => { ui.spinner(); ui.label("Restarting Signal Forge"); }
                _ => {}
            }
        });
        if approve {
            self.updater.approve();
        }
        if dismiss {
            if matches!(self.updater.state, UpdateState::Failed(_)) {
                self.updater.state = UpdateState::Deferred;
            } else {
                self.updater.defer();
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quiet_and_deferred_states_cannot_approve_or_download() {
        let mut controller = UpdateController::new(false, Arc::new(Mutex::new(None)));
        controller.approve();
        assert!(matches!(controller.state, UpdateState::Quiet));
        controller.state = UpdateState::Deferred;
        controller.approve();
        controller.poll();
        assert!(matches!(controller.state, UpdateState::Deferred));
    }
    #[test]
    fn newer_release_is_offered_once_and_not_now_never_downloads() {
        let mut controller = UpdateController::new(false, Arc::new(Mutex::new(None)));
        controller.state = UpdateState::Checking;
        let offer = Offer {
            version: semver::Version::new(1, 0, 0),
            notes_url: String::new(),
            archive: updater::Asset {
                name: String::new(),
                browser_download_url: String::new(),
            },
            checksum: updater::Asset {
                name: String::new(),
                browser_download_url: String::new(),
            },
        };
        controller
            .sender
            .send(Event::Checked(Ok(Some(offer.clone()))))
            .unwrap();
        controller.poll();
        assert!(matches!(controller.state, UpdateState::Available(_)));
        assert!(controller.receiver.try_recv().is_err());
        controller.defer();
        controller.approve();
        controller
            .sender
            .send(Event::Checked(Ok(Some(offer))))
            .unwrap();
        controller.poll();
        assert!(matches!(controller.state, UpdateState::Deferred));
        assert!(controller.prepared.is_none());
        assert!(controller.restart.lock().unwrap().is_none());
    }
    #[test]
    fn checked_current_or_offline_stays_quiet_and_prepare_failure_allows_continue() {
        let mut controller = UpdateController::new(false, Arc::new(Mutex::new(None)));
        controller.sender.send(Event::Checked(Ok(None))).unwrap();
        controller.poll();
        assert!(matches!(controller.state, UpdateState::Quiet));
        controller
            .sender
            .send(Event::Checked(Err("offline".into())))
            .unwrap();
        controller.poll();
        assert!(matches!(controller.state, UpdateState::Quiet));
        controller.state = UpdateState::Working(Progress::Verifying);
        controller
            .sender
            .send(Event::Prepared(Err("checksum mismatch".into())))
            .unwrap();
        controller.poll();
        assert!(matches!(controller.state, UpdateState::Failed(_)));
        assert!(controller.restart.lock().unwrap().is_none());
    }
}
