//! small reusable widgets.

pub mod denoise_view;
pub mod eq_view;
pub mod gate_view;
pub mod knob;
pub mod meter;
pub mod plugin_view;

use egui::{Color32, RichText, Ui, vec2};

pub const GREEN: Color32 = Color32::from_rgb(0x2e, 0xa0, 0x43);
pub const GREY: Color32 = Color32::from_rgb(0x55, 0x58, 0x5e);
pub const AMBER: Color32 = Color32::from_rgb(0xd9, 0x8c, 0x1f);

/// big coloured on/off pill. returns true when clicked.
pub fn on_off(ui: &mut Ui, on: bool) -> bool {
    let (text, fill) = if on { ("ON", GREEN) } else { ("OFF", GREY) };
    let button = egui::Button::new(RichText::new(text).strong().color(Color32::WHITE))
        .fill(fill)
        .corner_radius(10.0)
        .min_size(vec2(46.0, 22.0));
    ui.add(button)
        .on_hover_text(if on {
            "click to bypass"
        } else {
            "click to enable"
        })
        .clicked()
}
