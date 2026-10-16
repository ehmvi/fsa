//! colours, fonts and egui visuals: warm ink, cream text and a single amber
//! signal colour, like the social preview.

use std::sync::Arc;

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Shadow, Stroke,
    TextStyle, Visuals,
};

pub const BG: Color32 = Color32::from_rgb(0x14, 0x13, 0x0f);
/// sunken areas: graph, text fields, slider rails
pub const DEEP: Color32 = Color32::from_rgb(0x0f, 0x0e, 0x0b);
/// buttons, combo boxes, popups
pub const SURFACE: Color32 = Color32::from_rgb(0x22, 0x20, 0x1b);
pub const SURFACE_HI: Color32 = Color32::from_rgb(0x2e, 0x2b, 0x25);
pub const GRID: Color32 = Color32::from_rgb(0x2b, 0x29, 0x25);
/// hairlines, frames, the 0 db line
pub const LINE: Color32 = Color32::from_rgb(0x3d, 0x3a, 0x34);
pub const INK: Color32 = Color32::from_rgb(0xef, 0xe9, 0xdd);
pub const MUTED: Color32 = Color32::from_rgb(0x8c, 0x85, 0x7a);
pub const FAINT: Color32 = Color32::from_rgb(0x5a, 0x55, 0x4d);
pub const SIGNAL: Color32 = Color32::from_rgb(0xf2, 0xa9, 0x3b);
/// errors, clipping, a missing plugin
pub const ALERT: Color32 = Color32::from_rgb(0xe8, 0x6a, 0x4f);

const BOLD: &str = "bold";

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
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
            (TextStyle::Body, FontId::proportional(13.0)),
            (TextStyle::Button, FontId::proportional(13.0)),
            (TextStyle::Heading, bold(22.0)),
            (TextStyle::Monospace, mono(12.5)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(8.0, 3.0);
        style.spacing.slider_rail_height = 4.0;
        style.visuals = visuals();
    });
}

fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    let r = CornerRadius::same(3);
    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_corner_radius = CornerRadius::same(4);
    v.menu_corner_radius = CornerRadius::same(4);
    v.window_shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(110),
    };
    v.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 10,
        spread: 0,
        color: Color32::from_black_alpha(90),
    };
    v.extreme_bg_color = DEEP;
    v.faint_bg_color = SURFACE;
    v.code_bg_color = DEEP;
    v.hyperlink_color = SIGNAL;
    v.warn_fg_color = SIGNAL;
    v.error_fg_color = ALERT;
    v.slider_trailing_fill = true;
    v.selection.bg_fill = SIGNAL.gamma_multiply(0.28);
    v.selection.stroke = Stroke::new(1.0, INK);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = BG;
    w.noninteractive.weak_bg_fill = BG;
    w.noninteractive.bg_stroke = Stroke::new(1.0, GRID);
    w.noninteractive.fg_stroke = Stroke::new(1.0, INK.gamma_multiply(0.86));
    w.noninteractive.corner_radius = r;

    w.inactive.bg_fill = SURFACE_HI;
    w.inactive.weak_bg_fill = SURFACE;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, INK);
    w.inactive.corner_radius = r;

    w.hovered.bg_fill = SURFACE_HI;
    w.hovered.weak_bg_fill = SURFACE_HI;
    w.hovered.bg_stroke = Stroke::new(1.0, LINE);
    w.hovered.fg_stroke = Stroke::new(1.5, INK);
    w.hovered.corner_radius = r;

    w.active.bg_fill = SIGNAL;
    w.active.weak_bg_fill = SURFACE_HI;
    w.active.bg_stroke = Stroke::new(1.0, SIGNAL);
    w.active.fg_stroke = Stroke::new(2.0, INK);
    w.active.corner_radius = r;

    w.open.bg_fill = SURFACE;
    w.open.weak_bg_fill = SURFACE_HI;
    w.open.bg_stroke = Stroke::new(1.0, LINE);
    w.open.fg_stroke = Stroke::new(1.0, INK);
    w.open.corner_radius = r;
    v
}
