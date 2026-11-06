//! colours, fonts and egui visuals: warm charcoal cards, soft cream text and
//! a single amber signal colour, like the social preview.

use std::sync::Arc;

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow, Stroke,
    TextStyle, Visuals,
};

/// window background, behind the cards
pub const BG: Color32 = Color32::from_rgb(0x12, 0x11, 0x0f);
/// panels
pub const CARD: Color32 = Color32::from_rgb(0x1d, 0x1c, 0x18);
/// card outlines, barely there
pub const BORDER: Color32 = Color32::from_rgb(0x2e, 0x2b, 0x26);
/// sunken areas inside a card: graph, meters, text fields, slider rails
pub const DEEP: Color32 = Color32::from_rgb(0x16, 0x15, 0x13);
/// buttons, combo boxes, popups
pub const SURFACE: Color32 = Color32::from_rgb(0x24, 0x22, 0x1e);
pub const SURFACE_HI: Color32 = Color32::from_rgb(0x2f, 0x2c, 0x27);
pub const GRID: Color32 = Color32::from_rgb(0x25, 0x23, 0x20);
/// hairlines, frames, the 0 db line
pub const LINE: Color32 = Color32::from_rgb(0x38, 0x35, 0x30);
pub const INK: Color32 = Color32::from_rgb(0xe8, 0xe2, 0xd6);
pub const MUTED: Color32 = Color32::from_rgb(0x96, 0x8f, 0x83);
pub const FAINT: Color32 = Color32::from_rgb(0x5e, 0x59, 0x51);
/// small secondary text: indices, scales, counts (readable where FAINT is not)
pub const SUBTLE: Color32 = Color32::from_rgb(0x8c, 0x86, 0x7a);
pub const SIGNAL: Color32 = Color32::from_rgb(0xf0, 0xa8, 0x3e);
/// the low end of meters and level bars
pub const SIGNAL_DIM: Color32 = Color32::from_rgb(0x9a, 0x6a, 0x2a);
/// the track of a switch that is on: calm, but still clearly amber
pub const SWITCH_ON: Color32 = Color32::from_rgb(0xad, 0x77, 0x2d);
/// errors, clipping, a missing plugin
pub const ALERT: Color32 = Color32::from_rgb(0xe5, 0x6b, 0x52);

pub const RADIUS: u8 = 7;
pub const CARD_RADIUS: u8 = 12;

const BOLD: &str = "bold";

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

/// a raised panel on the window background
pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CARD_RADIUS)
        .inner_margin(Margin::same(16))
}

/// a sunken group inside a card
pub fn well() -> egui::Frame {
    egui::Frame::new()
        .fill(DEEP)
        .stroke(Stroke::new(1.0, GRID))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(16, 14))
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        (
            "inter",
            &include_bytes!("../../assets/fonts/Inter-Regular.ttf")[..],
        ),
        (
            "inter-bold",
            &include_bytes!("../../assets/fonts/Inter-Bold.ttf")[..],
        ),
    ] {
        fonts
            .font_data
            .insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }
    // inter only has latin; egui's bundled fonts cover symbols and emoji
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    fonts
        .families
        .get_mut(&FontFamily::Proportional)
        .unwrap()
        .insert(0, "inter".into());
    let mut family = vec!["inter-bold".to_string()];
    family.extend(fallback);
    fonts.families.insert(FontFamily::Name(BOLD.into()), family);
    ctx.set_fonts(fonts);

    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(11.0)),
            (TextStyle::Body, FontId::proportional(13.5)),
            (TextStyle::Button, FontId::proportional(13.5)),
            (TextStyle::Heading, bold(20.0)),
            (TextStyle::Monospace, mono(12.5)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size = egui::vec2(40.0, 26.0);
        style.spacing.combo_height = 320.0;
        style.spacing.menu_margin = Margin::same(6);
        style.spacing.window_margin = Margin::same(16);
        style.spacing.slider_rail_height = 4.0;
        style.spacing.slider_width = 180.0;
        style.visuals = visuals();
    });
}

fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    let r = CornerRadius::same(RADIUS);
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_corner_radius = CornerRadius::same(CARD_RADIUS);
    v.menu_corner_radius = CornerRadius::same(10);
    v.window_shadow = Shadow {
        offset: [0, 10],
        blur: 30,
        spread: 0,
        color: Color32::from_black_alpha(120),
    };
    v.popup_shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(100),
    };
    v.extreme_bg_color = DEEP;
    v.faint_bg_color = SURFACE;
    v.code_bg_color = DEEP;
    v.hyperlink_color = SIGNAL;
    v.warn_fg_color = SIGNAL;
    v.error_fg_color = ALERT;
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.selection.bg_fill = SIGNAL.gamma_multiply(0.22);
    v.selection.stroke = Stroke::new(1.0, INK);
    v.indent_has_left_vline = false;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = CARD;
    w.noninteractive.weak_bg_fill = CARD;
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, INK.gamma_multiply(0.86));
    w.noninteractive.corner_radius = r;

    w.inactive.bg_fill = SURFACE_HI;
    w.inactive.weak_bg_fill = SURFACE;
    w.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0, INK);
    w.inactive.corner_radius = r;

    w.hovered.bg_fill = SURFACE_HI;
    w.hovered.weak_bg_fill = SURFACE_HI;
    w.hovered.bg_stroke = Stroke::new(1.0, LINE);
    w.hovered.fg_stroke = Stroke::new(1.5, INK);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = SIGNAL;
    w.active.weak_bg_fill = SURFACE_HI;
    w.active.bg_stroke = Stroke::new(1.0, SIGNAL.gamma_multiply(0.6));
    w.active.fg_stroke = Stroke::new(2.0, INK);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;

    w.open.bg_fill = SURFACE;
    w.open.weak_bg_fill = SURFACE_HI;
    w.open.bg_stroke = Stroke::new(1.0, LINE);
    w.open.fg_stroke = Stroke::new(1.0, INK);
    w.open.corner_radius = r;
    v
}
