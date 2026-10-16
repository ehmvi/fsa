//! small reusable widgets.

pub mod denoise_view;
pub mod eq_view;
pub mod gate_view;
pub mod knob;
pub mod meter;
pub mod plugin_view;
pub mod theme;

use egui::{RichText, Stroke, Ui, vec2};

use theme::{BG, LINE, MUTED, SIGNAL};

/// toggle pill: amber when on (solid or outlined), grey outline when off
pub fn pill(ui: &mut Ui, text: &str, on: bool, solid: bool, min_width: f32) -> egui::Response {
    let (fg, fill, stroke) = if on && solid {
        (BG, SIGNAL, Stroke::NONE)
    } else if on {
        (
            SIGNAL,
            SIGNAL.gamma_multiply(0.12),
            Stroke::new(1.0, SIGNAL),
        )
    } else {
        (MUTED, egui::Color32::TRANSPARENT, Stroke::new(1.0, LINE))
    };
    let button = egui::Button::new(RichText::new(text).font(theme::mono(11.5)).color(fg))
        .fill(fill)
        .stroke(stroke)
        .corner_radius(10.0)
        .min_size(vec2(min_width, 20.0));
    ui.add(button)
}

/// on/off pill of a chain item or band. returns true when clicked.
pub fn on_off(ui: &mut Ui, on: bool) -> bool {
    pill(ui, if on { "ON" } else { "OFF" }, on, false, 42.0)
        .on_hover_text(if on {
            "click to bypass"
        } else {
            "click to enable"
        })
        .clicked()
}

/// small uppercase label, as on the axes of the preview
pub fn caption(ui: &mut Ui, text: &str) -> egui::Response {
    ui.label(
        RichText::new(text.to_uppercase())
            .font(theme::mono(11.0))
            .color(MUTED),
    )
}

/// panel title: bold name over a short amber rule
pub fn title(ui: &mut Ui, text: &str, dimmed: bool) {
    let color = if dimmed { MUTED } else { theme::INK };
    ui.label(RichText::new(text).font(theme::bold(24.0)).color(color));
    let (rect, _) = ui.allocate_exact_size(vec2(32.0, 3.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 0.0, if dimmed { LINE } else { SIGNAL });
}
