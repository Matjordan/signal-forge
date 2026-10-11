//! Contextual help registers targets, while one owner handles timing and navigation.
use super::{theme, *};
use signal_forge::help::{self, HoverTimer};

#[derive(Clone)]
struct Target {
    widget: egui::Id,
    topic: &'static str,
    rect: egui::Rect,
}
#[derive(Clone, Default)]
struct HelpFrame {
    hovered: Option<Target>,
    seen: std::collections::HashSet<egui::Id>,
    expanded: Option<egui::Id>,
    enabled: bool,
    interacting: bool,
}
fn frame_id() -> egui::Id {
    egui::Id::new("contextual-help-frame")
}

pub(super) fn explain(response: egui::Response, id: &'static str) -> egui::Response {
    let Some(topic) = help::topic(id) else {
        return response;
    };
    let expanded = response.ctx.data_mut(|data| {
        let frame = data.get_temp_mut_or_default::<HelpFrame>(frame_id());
        frame.seen.insert(response.id);
        if response.hovered() && response.enabled() && frame.enabled {
            frame.hovered = Some(Target {
                widget: response.id,
                topic: id,
                rect: response.rect,
            });
        }
        frame.expanded == Some(response.id)
    });
    if expanded {
        response
    } else {
        response.on_hover_text(topic.tooltip)
    }
}

/// Explicit UI keys map to stable documentation IDs; dynamic endpoint prefixes are ignored.
pub(super) fn topic_for_control(key: &str) -> Option<&'static str> {
    let label = key.rsplit(':').next().unwrap_or(key);
    Some(match label {
        "+ New Port" | "+ Open a port" => "serial.settings",
        "Virtual Pair" | "+ Virtual pair" => "virtual_pair.create",
        "Emulated baud" => "virtual_pair.timing",
        "Bridge" | "+ Bridge" => "bridge.create",
        "2 Tiles" | "4 Tiles" | "Save Workspace" | "Workspace…" => "workspace.save",
        "Replay…" => "capture.replay",
        "Network…" | "Network host" | "Network bind" | "Open network" | "UDP reply target"
        | "Set UDP target" => "network.setup",
        "Session…" | "Session (active)" => "session.manage",
        "Presets…" | "Presets" => "tx.presets",
        "Line" | "Raw Chunks" | "Hex" => "terminal.modes",
        "Send" | "Send payload" => "tx.payload",
        "Repeat" => "tx.repeat",
        "Files / RX" | "Record RX" | "Stop recording" => "recording.rx",
        "Triggered capture" => "capture.trigger",
        "Skip first byte" => "tx.checksum.skip_first",
        "Checksum options" | "Checksum XOR" | "Checksum Off" | "Checksum Hex" | "Checksum *Hex"
        | "Checksum Raw" => "tx.checksum",
        "Copy ASCII" | "Copy Hex" | "Copy XOR" => "selection.inspector",
        _ => return None,
    })
}

pub(super) struct HelpUi {
    pub open: bool,
    topic: &'static str,
    history: Vec<&'static str>,
    search: String,
    extended: bool,
    timer: HoverTimer,
    active: Option<Target>,
    popup_rect: Option<egui::Rect>,
    last_on_target: f64,
    focus_search: bool,
    scope: u16,
    reveal_topic: bool,
}
impl Default for HelpUi {
    fn default() -> Self {
        Self {
            open: false,
            topic: "getting.started",
            history: Vec::new(),
            search: String::new(),
            extended: true,
            timer: HoverTimer::default(),
            active: None,
            popup_rect: None,
            last_on_target: 0.,
            focus_search: false,
            scope: 0,
            reveal_topic: false,
        }
    }
}
impl HelpUi {
    pub fn open_topic(&mut self, id: &'static str) {
        if help::topic(id).is_none() {
            return;
        }
        if self.topic != id {
            self.history.push(self.topic);
            self.topic = id;
        }
        if !self.open {
            self.focus_search = true;
        }
        self.open = true;
        self.reveal_topic = true;
        self.search.clear();
        self.active = None;
        self.popup_rect = None;
        self.timer.reset();
        log::debug!("Help topic opened: {id}");
    }
    pub fn open_current(&mut self) {
        self.open_topic(self.topic);
    }
    pub fn begin_frame(&mut self, ctx: &egui::Context) {
        // Capture before terminal editors/shortcuts consume their key events.
        let interacting = ctx.input(|i| {
            i.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key { pressed: true, .. }
                        | egui::Event::Text(_)
                        | egui::Event::MouseWheel { .. }
                )
            })
        });
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F1)) {
            self.open_topic(self.topic);
        }
        if self.open && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.open = false;
        }
        if self.open && ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowLeft))
        {
            self.back();
        }
        if self.open && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::F)) {
            self.focus_search = true;
        }
        ctx.data_mut(|data| {
            data.insert_temp(
                frame_id(),
                HelpFrame {
                    hovered: None,
                    seen: Default::default(),
                    expanded: self.active.as_ref().map(|target| target.widget),
                    enabled: self.extended && !self.open,
                    interacting,
                },
            )
        });
    }
    fn back(&mut self) {
        if let Some(id) = self.history.pop() {
            self.topic = id;
            self.reveal_topic = true;
            self.search.clear();
            log::debug!("Help topic opened: {id}");
        }
    }
    pub fn show(&mut self, ctx: &egui::Context, scope: u16) {
        if self.scope != scope {
            self.scope = scope;
            self.active = None;
            self.popup_rect = None;
            self.timer.reset();
        }
        self.contextual(ctx);
        if !self.open {
            return;
        }
        let mut open = self.open;
        let screen = ctx.screen_rect();
        // Window resize limits describe its inner content. Reserve title/frame space.
        let guide = egui::Window::new("Help · Signal Forge")
            .id(egui::Id::new("user-guide"))
            .order(egui::Order::Foreground)
            .open(&mut open)
            .default_size([850.0, 640.0])
            .min_size([580.0, 320.0])
            .max_size([screen.width() - 80.0, screen.height() - 140.0])
            .collapsible(false)
            .show(ctx, |ui| self.guide_contents(ui, screen));
        if let Some(guide) = guide {
            let rect = guide.response.rect;
            log::debug!(
                "Help guide bounds: {},{},{},{}",
                rect.min.x,
                rect.min.y,
                rect.max.x,
                rect.max.y
            );
            ctx.move_to_top(guide.response.layer_id);
        }
        self.open = open;
    }

    fn guide_contents(&mut self, ui: &mut egui::Ui, screen: egui::Rect) {
        ui.horizontal_wrapped(|ui| {
            let back = ui.add_enabled(!self.history.is_empty(), egui::Button::new("Back"));
            theme::trace_control(&back, "Help Back");
            if back.clicked() { self.back(); }
            let search = theme::control(ui, "Help search",
                egui::TextEdit::singleline(&mut self.search)
                    .id(egui::Id::new("help-search"))
                    .hint_text("Search guide · Ctrl+F")
                    .desired_width(260.0));
            if self.focus_search { search.request_focus(); self.focus_search = false; }
            if theme::control(ui, "Clear help search", egui::Button::new("Clear")).clicked() {
                self.search.clear();
            }
            ui.checkbox(&mut self.extended, "Extended hover help")
                .on_hover_text("Ten-second explanations; this preference lasts until you quit. Short tooltips and the guide stay available.");
        });
        ui.separator();
        let max_height = (screen.height() - 230.0).clamp(220.0, 480.0);
        let height = (ui.available_height() - 25.0).clamp(220.0, max_height);
        ui.horizontal_top(|ui| {
            let nav_width = (ui.available_width() * 0.28).clamp(160.0, 240.0);
            ui.allocate_ui_with_layout(
                egui::vec2(nav_width, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.navigation(ui, height),
            );
            // A vertical Separator fills the parent's height and would expand
            // the window past its intended content size; bound this divider.
            let (divider, _) =
                ui.allocate_exact_size(egui::vec2(1.0, height), egui::Sense::hover());
            ui.painter().line_segment(
                [divider.left_top(), divider.left_bottom()],
                egui::Stroke::new(1.0_f32, theme::BORDER),
            );
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.article(ui, height),
            );
        });
        ui.separator();
        ui.small("Offline guide · F1 opens · Esc closes · Alt+Left goes back");
    }

    fn navigation(&mut self, ui: &mut egui::Ui, height: f32) {
        let reveal = std::mem::take(&mut self.reveal_topic);
        ui.label(egui::RichText::new("Topics").strong());
        let matches: Vec<_> = help::topics()
            .iter()
            .filter(|topic| topic.matches(&self.search))
            .collect();
        if matches.is_empty() {
            ui.weak("No matching topics. Try a device error or control name.");
        }
        egui::ScrollArea::vertical()
            .id_salt("help-navigation")
            .max_height(height - 30.0)
            .show(ui, |ui| {
                for topic in matches {
                    let response = theme::control(
                        ui,
                        &format!("Help topic {}", topic.id),
                        egui::Button::new(topic.title)
                            .selected(topic.id == self.topic)
                            .wrap(),
                    );
                    if reveal && topic.id == self.topic {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                    if response.clicked() {
                        self.open_topic(topic.id);
                    }
                }
            });
    }

    fn article(&mut self, ui: &mut egui::Ui, height: f32) {
        let topic = help::topic(self.topic).expect("Known guide location");
        ui.heading(topic.title);
        ui.label(egui::RichText::new(topic.tooltip).color(theme::MUTED));
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt(("help-body", self.topic))
            .max_height(height - 90.0)
            .show(ui, |ui| {
                if let Some(id) = render_body(ui, topic.body) {
                    self.open_topic(id);
                }
            });
    }

    fn contextual(&mut self, ctx: &egui::Context) {
        if self.open || !self.extended {
            self.active = None;
            self.popup_rect = None;
            self.timer.reset();
            return;
        }
        let frame = ctx.data_mut(|data| data.get_temp::<HelpFrame>(frame_id()).unwrap_or_default());
        if self
            .active
            .as_ref()
            .is_some_and(|target| !frame.seen.contains(&target.widget))
        {
            self.active = None;
            self.popup_rect = None;
            self.timer.reset();
        }
        let now = ctx.input(|i| i.time);
        let key_input = frame.interacting;
        let pressed = ctx.input(|i| i.pointer.any_pressed());
        let pointer = ctx.pointer_hover_pos();
        let over_popup = pointer.is_some_and(|p| {
            self.popup_rect
                .is_some_and(|rect| rect.expand(4.).contains(p))
        });
        if key_input || (pressed && !over_popup) {
            self.timer.dismiss();
            self.active = None;
            self.popup_rect = None;
        }
        let target = if let Some(target) = frame.hovered {
            self.last_on_target = now;
            Some(target)
        } else if self.active.is_some() && (over_popup || now - self.last_on_target < 0.25) {
            self.active.clone()
        } else {
            None
        };
        if self.timer.update(
            now,
            target.as_ref().map(|t| t.widget.value()),
            key_input || (pressed && !over_popup),
        ) {
            self.active = target;
        } else {
            self.active = None;
            self.popup_rect = None;
        }
        let Some(target) = self.active.clone() else {
            return;
        };
        let topic = help::topic(target.topic).unwrap();
        let mut dismiss = false;
        let mut learn = false;
        let response = egui::Area::new(egui::Id::new("expanded-context-help"))
            .order(egui::Order::Foreground)
            .fixed_pos(target.rect.left_bottom() + egui::vec2(0., 4.))
            .constrain(true)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(330.);
                    ui.label(egui::RichText::new(topic.title).strong());
                    inline(ui, topic.expanded);
                    ui.horizontal(|ui| {
                        learn = theme::button(ui, "Learn more").clicked();
                        dismiss = theme::button(ui, "Dismiss help").clicked();
                    });
                });
            });
        self.popup_rect = Some(response.response.rect);
        if learn {
            self.open_topic(topic.id);
        } else if dismiss {
            self.timer.dismiss();
            self.active = None;
            self.popup_rect = None;
        }
    }
}

fn render_body(ui: &mut egui::Ui, body: &str) -> Option<&'static str> {
    let mut selected = None;
    for paragraph in body.split("\n\n") {
        if paragraph.starts_with("Related:") {
            ui.add_space(8.);
            ui.label(egui::RichText::new("Related topics").strong());
            ui.horizontal_wrapped(|ui| {
                for link in paragraph.split("](#").skip(1) {
                    if let Some(topic) = link
                        .split_once(')')
                        .and_then(|(anchor, _)| help::by_anchor(anchor))
                    {
                        if theme::control(
                            ui,
                            &format!("Help related {}", topic.id),
                            egui::Button::new(topic.title),
                        )
                        .clicked()
                        {
                            selected = Some(topic.id);
                        }
                    }
                }
            });
        } else if paragraph
            .lines()
            .next()
            .is_some_and(|line| line.starts_with("1. "))
        {
            for line in paragraph.lines() {
                inline(ui, line);
                ui.add_space(4.);
            }
        } else {
            inline(ui, &paragraph.replace('\n', " "));
        }
        ui.add_space(8.);
    }
    selected
}
fn inline(ui: &mut egui::Ui, text: &str) {
    let mut job = egui::text::LayoutJob::default();
    let mut remaining = text;
    while !remaining.is_empty() {
        let marker = remaining.find(['`', '*']);
        let Some(start) = marker else {
            job.append(remaining, 0., format(false, false));
            break;
        };
        job.append(&remaining[..start], 0., format(false, false));
        let tail = &remaining[start..];
        let (delimiter, code) = if tail.starts_with('`') {
            ("`", true)
        } else if tail.starts_with("**") {
            ("**", false)
        } else {
            job.append("*", 0., format(false, false));
            remaining = &tail[1..];
            continue;
        };
        if let Some(end) = tail[delimiter.len()..].find(delimiter) {
            let content = &tail[delimiter.len()..delimiter.len() + end];
            job.append(content, 0., format(code, !code));
            remaining = &tail[delimiter.len() * 2 + end..];
        } else {
            job.append(tail, 0., format(false, false));
            break;
        }
    }
    ui.add(egui::Label::new(job).wrap().selectable(false));
}
fn format(code: bool, strong: bool) -> egui::TextFormat {
    egui::TextFormat {
        font_id: egui::FontId::new(
            14.,
            if code {
                egui::FontFamily::Monospace
            } else {
                egui::FontFamily::Proportional
            },
        ),
        color: if strong { theme::ACCENT } else { theme::TEXT },
        background: if code {
            theme::CANVAS
        } else {
            egui::Color32::TRANSPARENT
        },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(
        ctx: &egui::Context,
        help: &mut HelpUi,
        time: f64,
        events: Vec<egui::Event>,
        target: bool,
        blocked: bool,
    ) -> egui::Rect {
        let mut rect = egui::Rect::NOTHING;
        let _ = ctx.run(
            egui::RawInput {
                time: Some(time),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900., 600.),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                help.begin_frame(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut String::new())
                            .id(egui::Id::new("retained-editor")),
                    );
                    if target {
                        ui.add_space(475.);
                        ui.horizontal(|ui| {
                            ui.add_space(700.);
                            rect = theme::control(
                                ui,
                                "Skip first byte",
                                egui::Button::new("Skip first byte"),
                            )
                            .rect;
                        });
                    }
                });
                ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                help.show(ctx, u16::from(blocked));
            },
        );
        rect
    }
    fn hover(ctx: &egui::Context, help: &mut HelpUi) {
        let rect = frame(ctx, help, 0., vec![], true, false);
        frame(
            ctx,
            help,
            0.1,
            vec![egui::Event::PointerMoved(rect.center())],
            true,
            false,
        );
        frame(ctx, help, 0.2, vec![], true, false);
    }
    #[test]
    fn expanded_help_is_clamped_and_does_not_steal_editor_focus() {
        let ctx = egui::Context::default();
        let mut help = HelpUi::default();
        hover(&ctx, &mut help);
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("retained-editor")));
        frame(&ctx, &mut help, 9.9, vec![], true, false);
        assert!(help.active.is_none());
        frame(&ctx, &mut help, 10.3, vec![], true, false);
        frame(&ctx, &mut help, 10.4, vec![], true, false);
        assert_eq!(
            help.active.as_ref().unwrap().topic,
            "tx.checksum.skip_first"
        );
        let popup = help.popup_rect.unwrap();
        assert!(ctx.screen_rect().contains_rect(popup), "{popup:?}");
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(egui::Id::new("retained-editor"))
        );
        frame(
            &ctx,
            &mut help,
            10.5,
            vec![egui::Event::PointerMoved(popup.center())],
            true,
            false,
        );
        assert!(
            help.active.is_some(),
            "Popup must remain available for Learn more"
        );
    }
    #[test]
    fn disappearance_modal_and_keyboard_interaction_cancel_expanded_help() {
        for invalidation in 0..4 {
            let ctx = egui::Context::default();
            let mut help = HelpUi::default();
            hover(&ctx, &mut help);
            frame(&ctx, &mut help, 10.3, vec![], true, false);
            assert!(help.active.is_some());
            let events = if invalidation == 2 {
                vec![egui::Event::Text("a".into())]
            } else if invalidation == 3 {
                vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }]
            } else {
                vec![]
            };
            frame(
                &ctx,
                &mut help,
                10.4,
                events,
                invalidation != 0,
                invalidation == 1,
            );
            assert!(help.active.is_none());
            assert!(help.popup_rect.is_none());
        }
    }
    #[test]
    fn guide_navigation_handles_unknown_ids_and_remembers_current_topic() {
        let mut help = HelpUi::default();
        help.open_topic("tx.checksum.skip_first");
        help.open_topic("troubleshoot.checksum");
        help.open_topic("does.not.exist");
        assert_eq!(help.topic, "troubleshoot.checksum");
        help.back();
        assert_eq!(help.topic, "tx.checksum.skip_first");
        help.open = false;
        help.open_current();
        assert_eq!(help.topic, "tx.checksum.skip_first");
    }
    #[test]
    fn guide_fits_the_minimum_viewport_and_extended_help_can_be_disabled() {
        let ctx = egui::Context::default();
        let mut help = HelpUi::default();
        help.open_topic("troubleshoot.virtual");
        for time in [0.0, 0.1, 0.2] {
            frame(&ctx, &mut help, time, vec![], false, false);
        }
        let rect = ctx
            .memory(|memory| memory.area_rect(egui::Id::new("user-guide")))
            .unwrap();
        assert!(ctx.screen_rect().contains_rect(rect), "{rect:?}");
        help.open = false;
        help.extended = false;
        hover(&ctx, &mut help);
        frame(&ctx, &mut help, 11.0, vec![], true, false);
        assert!(help.active.is_none());
        help.open_current();
        assert!(help.open);
    }

    #[test]
    fn all_control_topic_ids_resolve() {
        for key in [
            "+ New Port",
            "Virtual Pair",
            "Emulated baud",
            "Bridge",
            "4 Tiles",
            "Replay…",
            "Session…",
            "Presets",
            "serial:/dev/pts/0:Hex",
            "serial:/dev/pts/0:Send payload",
            "Repeat",
            "Files / RX",
            "Triggered capture",
            "Skip first byte",
            "Checksum options",
            "Copy XOR",
        ] {
            assert!(
                help::topic(topic_for_control(key).unwrap()).is_some(),
                "{key}"
            );
        }
    }
}
