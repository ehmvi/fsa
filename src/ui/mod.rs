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

/// quiet outlined text button. `danger` turns the label red on hover.
pub fn ghost(ui: &mut Ui, text: &str, enabled: bool, danger: bool) -> egui::Response {
    let font = egui::FontId::proportional(12.5);
    let width = ui
        .painter()
        .layout_no_wrap(text.to_string(), font.clone(), MUTED)
        .size()
        .x;
    let size = vec2(width + 20.0, 24.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let hovered = enabled && resp.hovered();
    let color = match (enabled, hovered, danger) {
        (false, _, _) => FAINT,
        (true, true, true) => theme::ALERT,
        (true, true, false) => INK,
        (true, false, _) => MUTED,
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        12.0,
        if hovered {
            theme::SURFACE
        } else {
            Color32::TRANSPARENT
        },
        Stroke::new(1.0, if enabled { LINE } else { theme::BORDER }),
        egui::StrokeKind::Inside,
    );
    let galley = painter.layout_no_wrap(text.to_string(), font, color);
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
    resp
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
        // a muted amber track; the cream thumb carries the "on"
        lerp_color(SURFACE_HI, theme::SIGNAL_DIM, t)
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
    chip(ui, text, lit, 84.0, Sense::hover())
}

/// the chip of `state_chip`, with a minimum width and any sense
pub fn chip(ui: &mut Ui, text: &str, lit: bool, min_width: f32, sense: Sense) -> egui::Response {
    let color = if lit { SIGNAL } else { MUTED };
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), theme::mono(11.0), color);
    let size = vec2((galley.size().x + 32.0).max(min_width), 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let hovered = resp.hovered() && sense.senses_click();
    let painter = ui.painter();
    painter.rect(
        rect,
        13.0,
        match (lit, hovered) {
            (true, false) => SIGNAL.gamma_multiply(0.12),
            (true, true) => SIGNAL.gamma_multiply(0.2),
            (false, false) => Color32::TRANSPARENT,
            (false, true) => theme::SURFACE,
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

/// knob radius in the gate and denoise panels
pub const PANEL_KNOB: f32 = 28.0;

/// lays knobs out in equal columns (`columns` of them across the row), so the
/// gate and denoise panels share one grid
pub fn knob_columns(
    ui: &mut Ui,
    columns: usize,
    count: usize,
    mut add: impl FnMut(&mut Ui, usize),
) {
    let pitch = knob_pitch(ui, columns);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for i in 0..count {
            knob_cell(ui, pitch, |ui| add(ui, i));
        }
    });
}

/// column width of `knob_columns` for the current row
pub fn knob_pitch(ui: &Ui, columns: usize) -> f32 {
    (ui.available_width() / columns as f32).clamp(80.0, 132.0)
}

/// one column of `knob_columns`, for rows that mix knobs with other things
pub fn knob_cell(ui: &mut Ui, pitch: f32, add: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(
        vec2(pitch, 2.0 * PANEL_KNOB + 40.0),
        egui::Layout::top_down(egui::Align::Center),
        add,
    );
}

/// a state chip, a level bar and a right-aligned readout on one line
pub fn status_strip(
    ui: &mut Ui,
    chip_text: &str,
    chip_lit: bool,
    bar: (f32, Option<f32>, bool),
    readout: &str,
) -> egui::Response {
    ui.horizontal(|ui| {
        let chip = state_chip(ui, chip_text, chip_lit);
        ui.add_space(10.0);
        let right = ui.max_rect().right();
        let w = (right - ui.cursor().left() - 64.0).max(80.0);
        let rect = level_bar(ui, w, bar.0, bar.1, bar.2);
        ui.painter().text(
            pos2(right, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            readout,
            theme::mono(11.0),
            MUTED,
        );
        chip
    })
    .inner
}

/// horizontal level bar. `t` and `marker` are 0..1; the fill is amber when
/// `lit`, dim amber otherwise, and the marker (a threshold) is a cream tick.
pub fn level_bar(ui: &mut Ui, width: f32, t: f32, marker: Option<f32>, lit: bool) -> egui::Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 10.0), Sense::hover());
    let painter = ui.painter();
    painter.rect(
        rect,
        5.0,
        theme::BG,
        Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );
    let inner = rect.shrink(2.0);
    let w = inner.width() * t.clamp(0.0, 1.0);
    // nothing rather than a dot when there's (almost) no signal
    if w >= inner.height() {
        let fill = egui::Rect::from_min_max(inner.min, pos2(inner.left() + w, inner.bottom()));
        painter.rect_filled(fill, 3.0, if lit { SIGNAL } else { theme::SIGNAL_DIM });
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
