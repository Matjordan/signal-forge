//! Signal Forge's shared palette and compact instrument controls.
use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};

pub const BACKGROUND: Color32 = Color32::from_rgb(12, 20, 28);
pub const PANEL: Color32 = Color32::from_rgb(17, 27, 38);
pub const CARD: Color32 = Color32::from_rgb(20, 30, 42);
pub const CANVAS: Color32 = Color32::from_rgb(7, 13, 20);
pub const TEXT: Color32 = Color32::from_rgb(215, 227, 242);
pub const MUTED: Color32 = Color32::from_rgb(160, 173, 189);
pub const BORDER: Color32 = Color32::from_rgb(41, 55, 71);
pub const ACCENT: Color32 = Color32::from_rgb(43, 145, 246);
pub const PRIMARY: Color32 = Color32::from_rgb(21, 99, 218);
pub const SELECTION: Color32 = Color32::from_rgb(28, 76, 128);
pub const CONNECTED: Color32 = Color32::from_rgb(89, 210, 118);
pub const WARNING: Color32 = Color32::from_rgb(255, 210, 122);
pub const ERROR: Color32 = Color32::from_rgb(255, 141, 153);
pub const DANGER: Color32 = Color32::from_rgb(140, 40, 50);
pub const RX: Color32 = Color32::from_rgb(128, 205, 141);
pub const TX: Color32 = Color32::from_rgb(60, 158, 246);

pub const CONTROL_HEIGHT: f32 = 18.0;
pub const CANVAS_PADDING: f32 = 6.0;
pub const TRAFFIC_ROW_HEIGHT: f32 = 18.0;
pub const SEND_AREA_HEIGHT: f32 = 205.0;
const CORNER_RADIUS: u8 = 3;

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    let visuals = &mut style.visuals;
    visuals.panel_fill = PANEL;
    visuals.window_fill = CARD;
    visuals.extreme_bg_color = CANVAS;
    visuals.faint_bg_color = CARD;
    visuals.code_bg_color = CANVAS;
    visuals.hyperlink_color = ACCENT;
    visuals.warn_fg_color = WARNING;
    visuals.error_fg_color = ERROR;
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.window_corner_radius = CornerRadius::same(CORNER_RADIUS);
    visuals.menu_corner_radius = CornerRadius::same(CORNER_RADIUS);
    visuals.selection.bg_fill = SELECTION;
    visuals.selection.stroke = Stroke::new(1.0, TEXT);
    for (widget, background) in [
        (&mut visuals.widgets.noninteractive, PANEL),
        (&mut visuals.widgets.inactive, CARD),
        (&mut visuals.widgets.hovered, SELECTION),
        (&mut visuals.widgets.active, PRIMARY),
        (&mut visuals.widgets.open, CARD),
    ] {
        widget.bg_fill = background;
        widget.weak_bg_fill = background;
        widget.bg_stroke = Stroke::new(1.0, BORDER);
        widget.fg_stroke = Stroke::new(1.0, TEXT);
        widget.corner_radius = CornerRadius::same(CORNER_RADIUS);
    }
    style.spacing.item_spacing = egui::vec2(8.0, 3.0);
    style.spacing.button_padding = egui::vec2(4.0, 1.0);
    style.spacing.interact_size.y = CONTROL_HEIGHT;
    style.spacing.window_margin = egui::Margin::same(6);
    style.spacing.menu_margin = egui::Margin::same(6);
    for (text_style, size, family) in [
        (TextStyle::Heading, 18.0, FontFamily::Proportional),
        (TextStyle::Body, 12.5, FontFamily::Proportional),
        (TextStyle::Button, 12.5, FontFamily::Proportional),
        (TextStyle::Small, 9.0, FontFamily::Proportional),
        (TextStyle::Monospace, 12.0, FontFamily::Monospace),
    ] {
        style
            .text_styles
            .insert(text_style, FontId::new(size, family));
    }
    ctx.set_style(style);
}

pub fn primary_button(label: impl Into<egui::WidgetText>) -> egui::Button<'static> {
    egui::Button::new(label).fill(PRIMARY)
}

pub fn danger_button(label: impl Into<egui::WidgetText>) -> egui::Button<'static> {
    egui::Button::new(label).fill(DANGER)
}

pub fn canvas_frame() -> egui::Frame {
    egui::Frame::new().fill(CANVAS).inner_margin(CANVAS_PADDING)
}
