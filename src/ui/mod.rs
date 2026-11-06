//! small reusable widgets.

pub mod denoise_view;
pub mod eq_view;
pub mod gate_view;
pub mod knob;
pub mod meter;
pub mod plugin_view;
pub mod theme;

use egui::{Color32, RichText, Sense, Stroke, Ui, pos2, vec2};

use theme::{FAINT, INK, LINE, MUTED, SIGNAL, SURFACE_HI};

/// height of ghost buttons, and of the text fields next to them
pub const CONTROL_H: f32 = 26.0;

/// quiet outlined text button. `danger` turns the label red on hover.
pub fn ghost(ui: &mut Ui, text: &str, enabled: bool, danger: bool) -> egui::Response {
    ghost_impl(ui, text, enabled, danger, false)
}

/// a ghost button that stays tinted amber while `on` (solo, dynamic)
pub fn ghost_toggle(ui: &mut Ui, text: &str, on: bool, enabled: bool) -> egui::Response {
    ghost_impl(ui, text, enabled, false, on)
}

fn ghost_impl(ui: &mut Ui, text: &str, enabled: bool, danger: bool, on: bool) -> egui::Response {
    let font = egui::FontId::proportional(12.5);
    let width = ui
        .painter()
        .layout_no_wrap(text.to_string(), font.clone(), MUTED)
        .size()
        .x;
    let size = vec2(width + 22.0, CONTROL_H);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let hovered = enabled && resp.hovered();
    let color = match (enabled, on, hovered, danger) {
        (false, _, _, _) => FAINT,
        (true, true, _, _) => SIGNAL,
        (true, false, true, true) => theme::ALERT,
        (true, false, true, false) => INK,
        (true, false, false, _) => MUTED,
    };
    let (fill, stroke) = match (on && enabled, hovered) {
        (true, false) => (SIGNAL.gamma_multiply(0.12), SIGNAL.gamma_multiply(0.5)),
        (true, true) => (SIGNAL.gamma_multiply(0.2), SIGNAL.gamma_multiply(0.6)),
        (false, true) => (theme::SURFACE, LINE),
        (false, false) => (
            Color32::TRANSPARENT,
            if enabled { LINE } else { theme::BORDER },
        ),
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        CONTROL_H / 2.0,
        fill,
        Stroke::new(1.0, stroke),
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
        lerp_color(SURFACE_HI, theme::SWITCH_ON, t)
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
    chip(ui, text, lit, 96.0, Sense::hover())
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
pub const PANEL_KNOB: f32 = 32.0;

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
            knob_cell(ui, pitch, PANEL_KNOB, |ui| add(ui, i));
        }
    });
}

/// column width of `knob_columns` for the current row
pub fn knob_pitch(ui: &Ui, columns: usize) -> f32 {
    (ui.available_width() / columns as f32).clamp(80.0, 132.0)
}

/// one column of `knob_columns`, for rows that mix knobs with other things
pub fn knob_cell(ui: &mut Ui, pitch: f32, radius: f32, add: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(
        vec2(pitch, 2.0 * radius + 40.0),
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
    ticks: &[(f32, &str)],
) -> egui::Response {
    let resp = ui
        .horizontal(|ui| {
            let chip = state_chip(ui, chip_text, chip_lit);
            ui.add_space(12.0);
            let right = ui.max_rect().right();
            let w = (right - ui.cursor().left() - 64.0).max(80.0);
            let rect = level_bar_sized(ui, vec2(w, 12.0), bar.0, bar.1, bar.2);
            let painter = ui.painter();
            painter.text(
                pos2(right, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                readout,
                theme::mono(11.0),
                MUTED,
            );
            // scale under the bar, in the space reserved below the row
            let inner = rect.shrink(2.0);
            for &(t, label) in ticks {
                let align = if t <= 0.0 {
                    egui::Align2::LEFT_TOP
                } else if t >= 1.0 {
                    egui::Align2::RIGHT_TOP
                } else {
                    egui::Align2::CENTER_TOP
                };
                painter.text(
                    pos2(inner.left() + inner.width() * t, rect.bottom() + 8.0),
                    align,
                    label,
                    theme::mono(9.5),
                    theme::SUBTLE,
                );
            }
            chip
        })
        .inner;
    if !ticks.is_empty() {
        ui.add_space(12.0);
    }
    resp
}

/// a quiet hint pinned to the bottom of a panel, where the eq has its bottom
/// bar, so every panel has the same frame
pub fn panel_footer(ui: &mut Ui, text: &str) {
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        ui.add_space(4.0);
        ui.label(RichText::new(text).size(12.5).color(theme::SUBTLE));
    });
}

/// a well with the roomier padding of the gate and denoise panels
pub fn panel_well() -> egui::Frame {
    theme::well().inner_margin(egui::Margin::symmetric(20, 22))
}

/// a thin close cross, cream on hover
pub fn close_button(ui: &mut Ui) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
    let color = if resp.hovered() { INK } else { theme::SUBTLE };
    if resp.hovered() {
        ui.painter().rect_filled(rect, 12.0, theme::SURFACE);
    }
    let r = rect.shrink(7.5);
    let stroke = Stroke::new(1.5, color);
    ui.painter()
        .line_segment([r.left_top(), r.right_bottom()], stroke);
    ui.painter()
        .line_segment([r.right_top(), r.left_bottom()], stroke);
    resp
}

/// horizontal level bar. `t` and `marker` are 0..1; the fill is amber when
/// `lit`, dim amber otherwise, and the marker (a threshold) is a cream tick.
pub fn level_bar(ui: &mut Ui, width: f32, t: f32, marker: Option<f32>, lit: bool) -> egui::Rect {
    level_bar_sized(ui, vec2(width, 10.0), t, marker, lit)
}

fn level_bar_sized(
    ui: &mut Ui,
    size: egui::Vec2,
    t: f32,
    marker: Option<f32>,
    lit: bool,
) -> egui::Rect {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    painter.rect(
        rect,
        size.y / 2.0,
        theme::BG,
        Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );
    let inner = rect.shrink(2.0);
    let w = inner.width() * t.clamp(0.0, 1.0);
    // nothing rather than a dot when there's (almost) no signal
    if w >= inner.height() {
        let fill = egui::Rect::from_min_max(inner.min, pos2(inner.left() + w, inner.bottom()));
        painter.rect_filled(
            fill,
            inner.height() / 2.0,
            if lit { SIGNAL } else { theme::SIGNAL_DIM },
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
