//! Signal Forge's shared palette and compact instrument controls.
use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};

pub const BACKGROUND: Color32 = Color32::from_rgb(10, 16, 23);
pub const PANEL: Color32 = Color32::from_rgb(17, 25, 34);
pub const CARD: Color32 = Color32::from_rgb(23, 34, 46);
pub const CANVAS: Color32 = Color32::from_rgb(5, 9, 13);
pub const TEXT: Color32 = Color32::from_rgb(228, 235, 245);
pub const MUTED: Color32 = Color32::from_rgb(151, 170, 192);
pub const BORDER: Color32 = Color32::from_rgb(40, 58, 76);
pub const ACCENT: Color32 = Color32::from_rgb(34, 166, 255);
pub const PRIMARY: Color32 = Color32::from_rgb(22, 96, 225);
pub const SELECTION: Color32 = Color32::from_rgb(23, 51, 88);
pub const CONNECTED: Color32 = Color32::from_rgb(55, 206, 96);
pub const WARNING: Color32 = Color32::from_rgb(255, 210, 122);
pub const ERROR: Color32 = Color32::from_rgb(255, 141, 153);
pub const DANGER: Color32 = Color32::from_rgb(140, 40, 50);
pub const RX: Color32 = Color32::from_rgb(76, 224, 111);
pub const TX: Color32 = Color32::from_rgb(37, 185, 255);

pub const CONTROL_HEIGHT: f32 = 28.0;
pub const CANVAS_PADDING: f32 = 6.0;
pub const TRAFFIC_ROW_HEIGHT: f32 = 19.0;
pub const SEND_AREA_HEIGHT: f32 = 136.0;
const CORNER_RADIUS: u8 = 4;

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    let visuals = &mut style.visuals;
    visuals.panel_fill = PANEL;
    visuals.override_text_color = Some(TEXT);
    visuals.window_fill = CARD;
    visuals.extreme_bg_color = CANVAS;
    visuals.faint_bg_color = CARD;
    visuals.code_bg_color = CANVAS;
    visuals.hyperlink_color = ACCENT;
    visuals.warn_fg_color = WARNING;
    visuals.error_fg_color = ERROR;
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.window_corner_radius = CornerRadius::same(CORNER_RADIUS);
    visuals.menu_corner_radius = CornerRadius::same(CORNER_RADIUS);
    visuals.selection.bg_fill = SELECTION;
    visuals.selection.stroke = Stroke::new(1.0_f32, TEXT);
    for (widget, background) in [
        (&mut visuals.widgets.noninteractive, PANEL),
        (&mut visuals.widgets.inactive, CARD),
        (&mut visuals.widgets.hovered, SELECTION),
        (&mut visuals.widgets.active, PRIMARY),
        (&mut visuals.widgets.open, CARD),
    ] {
        widget.bg_fill = background;
        widget.weak_bg_fill = background;
        widget.bg_stroke = Stroke::new(1.0_f32, BORDER);
        widget.fg_stroke = Stroke::new(1.0_f32, TEXT);
        widget.corner_radius = CornerRadius::same(CORNER_RADIUS);
    }
    style.spacing.item_spacing = egui::vec2(8.0, 5.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size.y = CONTROL_HEIGHT;
    style.spacing.window_margin = egui::Margin::same(10);
    style.spacing.menu_margin = egui::Margin::same(10);
    for (text_style, size, family) in [
        (TextStyle::Heading, 16.0, FontFamily::Proportional),
        (TextStyle::Body, 14.0, FontFamily::Proportional),
        (TextStyle::Button, 13.0, FontFamily::Proportional),
        (TextStyle::Small, 11.0, FontFamily::Proportional),
        (TextStyle::Monospace, 13.0, FontFamily::Monospace),
    ] {
        style
            .text_styles
            .insert(text_style, FontId::new(size, family));
    }
    ctx.set_style(style);
}

pub fn primary_button(label: impl Into<egui::WidgetText>) -> egui::Button<'static> {
    egui::Button::new(label)
        .fill(PRIMARY)
        .min_size(egui::vec2(84.0, 30.0))
}

pub fn danger_button(label: impl Into<egui::WidgetText>) -> egui::Button<'static> {
    egui::Button::new(label)
        .fill(DANGER)
        .min_size(egui::vec2(70.0, 30.0))
}

pub fn canvas_frame() -> egui::Frame {
    egui::Frame::new().fill(CANVAS).inner_margin(CANVAS_PADDING)
}

pub fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(CornerRadius::same(CORNER_RADIUS))
        .inner_margin(6.0)
}

/// Debug geometry lets GUI smoke checks follow responsive layouts and display scale.
pub fn control(ui: &mut egui::Ui, key: &str, widget: impl egui::Widget) -> egui::Response {
    let response = ui.add(widget);
    trace_control(&response, key);
    response
}

pub fn button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    control(ui, label, egui::Button::new(label))
}

pub fn tab_value<T: PartialEq>(
    ui: &mut egui::Ui,
    current: &mut T,
    value: T,
    label: &str,
    key: &str,
) -> egui::Response {
    let selected = *current == value;
    let mut response = control(
        ui,
        key,
        egui::Button::new(label)
            .frame(false)
            .stroke(Stroke::NONE)
            .fill(if selected {
                SELECTION
            } else {
                Color32::TRANSPARENT
            })
            .corner_radius(CornerRadius::ZERO),
    );
    if selected {
        ui.painter().line_segment(
            [response.rect.left_bottom(), response.rect.right_bottom()],
            Stroke::new(2.0_f32, ACCENT),
        );
    }
    if response.clicked() {
        *current = value;
        response.mark_changed();
    }
    response
}

pub fn brand(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
    let points = [
        (0., 17.),
        (6., 17.),
        (10., 7.),
        (14., 28.),
        (18., 2.),
        (22., 25.),
        (26., 12.),
        (30., 17.),
        (34., 17.),
    ]
    .into_iter()
    .map(|(x, y)| rect.min + egui::vec2(x, y))
    .collect();
    ui.painter()
        .add(egui::Shape::line(points, Stroke::new(2.5_f32, ACCENT)));
    ui.vertical(|ui| {
        ui.label(egui::RichText::new("Signal Forge").strong().size(21.0));
        ui.label(egui::RichText::new("Serial Workbench").small().color(MUTED));
    });
}

pub fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 5.0, color);
}

pub fn trace_control(response: &egui::Response, key: &str) {
    let center = response.rect.center() * response.ctx.pixels_per_point();
    log::debug!("UI control {key}: {},{}", center.x, center.y);
}
