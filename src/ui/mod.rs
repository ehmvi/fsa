//! small reusable widgets.

pub mod denoise_view;
pub mod eq_view;
pub mod gate_view;
pub mod knob;
pub mod meter;
pub mod plugin_view;
pub mod theme;

use egui::{Color32, RichText, Sense, Stroke, Ui, pos2, vec2};

use theme::{BG, FAINT, INK, LINE, MUTED, SIGNAL, SURFACE_HI};

/// toggle pill: amber when on (solid or tinted), grey outline when off
pub fn pill(ui: &mut Ui, text: &str, on: bool, solid: bool, min_width: f32) -> egui::Response {
    let (fg, fill, stroke) = if on && solid {
        (BG, SIGNAL, Stroke::NONE)
    } else if on {
        (
            SIGNAL,
            SIGNAL.gamma_multiply(0.14),
            Stroke::new(1.0, SIGNAL.gamma_multiply(0.55)),
        )
    } else {
        (MUTED, Color32::TRANSPARENT, Stroke::new(1.0, LINE))
    };
    let button = egui::Button::new(RichText::new(text).font(theme::mono(11.0)).color(fg))
        .fill(fill)
        .stroke(stroke)
        .corner_radius(12.0)
        .min_size(vec2(min_width, 24.0));
    ui.add(button)
}

/// on/off switch. returns true when clicked.
pub fn on_off(ui: &mut Ui, on: bool) -> bool {
    switch(ui, on)
        .on_hover_text(if on {
            "click to bypass"
        } else {
            "click to enable"
        })
        .clicked()
}

/// a small sliding switch, amber when on
pub fn switch(ui: &mut Ui, on: bool) -> egui::Response {
    let size = vec2(30.0, 17.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let t = ui.ctx().animate_bool_responsive(resp.id, on);
    let enabled = ui.is_enabled();
    let track = if enabled {
        lerp_color(SURFACE_HI, SIGNAL.gamma_multiply(0.85), t)
    } else {
        SURFACE_HI.gamma_multiply(0.6)
    };
    let painter = ui.painter();
    let radius = rect.height() / 2.0;
    painter.rect(
        rect,
        radius,
        track,
        Stroke::new(1.0, if on { Color32::TRANSPARENT } else { LINE }),
        egui::StrokeKind::Inside,
    );
    let x = egui::lerp(rect.left() + radius..=rect.right() - radius, t);
    let knob = if !enabled {
        FAINT
    } else if resp.hovered() {
        Color32::WHITE
    } else {
        INK
    };
    painter.circle_filled(pos2(x, rect.center().y), radius - 3.0, knob);
    resp
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// small uppercase label, as on the axes of the preview
pub fn caption(ui: &mut Ui, text: &str) -> egui::Response {
    ui.label(
        RichText::new(text.to_uppercase())
            .font(theme::mono(10.5))
            .color(MUTED),
    )
}

/// panel title: bold name over a short amber rule
pub fn title(ui: &mut Ui, text: &str, dimmed: bool) {
    let color = if dimmed { MUTED } else { INK };
    ui.label(RichText::new(text).font(theme::bold(21.0)).color(color));
    let (rect, _) = ui.allocate_exact_size(vec2(28.0, 3.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, 1.5, if dimmed { LINE } else { SIGNAL });
}

/// a status dot with a short mono label, e.g. "● OPEN"
pub fn state_chip(ui: &mut Ui, text: &str, lit: bool) -> egui::Response {
    let color = if lit { SIGNAL } else { MUTED };
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), theme::mono(11.0), color);
    let size = vec2(galley.size().x + 32.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    painter.rect(
        rect,
        12.0,
        if lit {
            SIGNAL.gamma_multiply(0.12)
        } else {
            Color32::TRANSPARENT
        },
        Stroke::new(
            1.0,
            if lit {
                SIGNAL.gamma_multiply(0.45)
            } else {
                LINE
            },
        ),
        egui::StrokeKind::Inside,
    );
    let dot = pos2(rect.left() + 13.0, rect.center().y);
    if lit {
        painter.circle_filled(dot, 6.0, SIGNAL.gamma_multiply(0.25));
    }
    painter.circle_filled(dot, 3.5, if lit { SIGNAL } else { FAINT });
    painter.galley(
        pos2(rect.left() + 23.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    resp
}

/// horizontal level bar. `t` and `marker` are 0..1; the fill is amber when
/// `lit`, grey otherwise, and the marker (a threshold) is a cream tick.
pub fn level_bar(ui: &mut Ui, width: f32, t: f32, marker: Option<f32>, lit: bool) -> egui::Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 10.0), Sense::hover());
    let painter = ui.painter();
    painter.rect(
        rect,
        5.0,
        theme::DEEP,
        Stroke::new(1.0, theme::GRID),
        egui::StrokeKind::Inside,
    );
    let inner = rect.shrink(2.0);
    let t = t.clamp(0.0, 1.0);
    if t > 0.0 {
        let fill = egui::Rect::from_min_max(
            inner.min,
            pos2(
                inner.left() + (inner.width() * t).max(inner.height()),
                inner.bottom(),
            ),
        );
        painter.rect_filled(
            fill,
            3.0,
            if lit {
                SIGNAL
            } else {
                MUTED.gamma_multiply(0.6)
            },
        );
    }
    if let Some(m) = marker {
        let x = inner.left() + inner.width() * m.clamp(0.0, 1.0);
        painter.line_segment(
            [pos2(x, rect.top() - 3.0), pos2(x, rect.bottom() + 3.0)],
            Stroke::new(2.0, INK),
        );
    }
    rect
}
