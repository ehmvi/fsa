//! eq panel: response curve with draggable band handles, plus sliders.

use egui::{Color32, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::audio::SAMPLE_RATE;
use crate::dsp::biquad::FilterKind;
use crate::dsp::eq::{BANDS, EqParams, KINDS, NAMES};

const F_MIN: f32 = 20.0;
const F_MAX: f32 = 20_000.0;
const DB_RANGE: f32 = 18.0;
pub const BAND_COLORS: [Color32; BANDS] = [
    Color32::from_rgb(0xe0, 0x5a, 0x5a),
    Color32::from_rgb(0xe0, 0xa0, 0x40),
    Color32::from_rgb(0x6c, 0xc0, 0x5a),
    Color32::from_rgb(0x4a, 0xa8, 0xe0),
    Color32::from_rgb(0xb0, 0x7c, 0xe0),
];

fn x_of(rect: Rect, f: f32) -> f32 {
    let t = (f / F_MIN).ln() / (F_MAX / F_MIN).ln();
    rect.left() + t * rect.width()
}

fn f_of(rect: Rect, x: f32) -> f32 {
    let t = ((x - rect.left()) / rect.width()).clamp(0.0, 1.0);
    F_MIN * (F_MAX / F_MIN).powf(t)
}

fn y_of(rect: Rect, db: f32) -> f32 {
    rect.center().y - db / DB_RANGE * rect.height() * 0.5
}

fn db_of(rect: Rect, y: f32) -> f32 {
    ((rect.center().y - y) / (rect.height() * 0.5) * DB_RANGE).clamp(-DB_RANGE, DB_RANGE)
}

/// returns true if any parameter changed
pub fn show(ui: &mut Ui, params: &EqParams) -> bool {
    let mut changed = false;
    let fs = SAMPLE_RATE as f32;
    let height = (ui.available_height() * 0.5).clamp(160.0, 320.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_gray(22));

    // grid
    let grid = Stroke::new(1.0, Color32::from_gray(45));
    for f in [50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10_000.0] {
        let x = x_of(rect, f);
        painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], grid);
        let label = if f >= 1000.0 {
            format!("{}k", f / 1000.0)
        } else {
            format!("{f}")
        };
        painter.text(
            pos2(x + 2.0, rect.bottom() - 2.0),
            egui::Align2::LEFT_BOTTOM,
            label,
            egui::FontId::proportional(10.0),
            Color32::from_gray(110),
        );
    }
    for db in [-12.0, -6.0, 0.0, 6.0, 12.0] {
        let y = y_of(rect, db);
        let stroke = if db == 0.0 {
            Stroke::new(1.0, Color32::from_gray(80))
        } else {
            grid
        };
        painter.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], stroke);
        painter.text(
            pos2(rect.left() + 2.0, y - 1.0),
            egui::Align2::LEFT_BOTTOM,
            if db == 0.0 {
                "0".to_string()
            } else {
                format!("{db:+}")
            },
            egui::FontId::proportional(10.0),
            Color32::from_gray(110),
        );
    }

    // combined response
    let n = rect.width().max(2.0) as usize;
    let points: Vec<Pos2> = (0..=n)
        .map(|i| {
            let x = rect.left() + i as f32 * rect.width() / n as f32;
            let db = params
                .response_db(fs, f_of(rect, x))
                .clamp(-DB_RANGE * 1.5, DB_RANGE * 1.5);
            pos2(x, y_of(rect, db))
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        Stroke::new(2.0, Color32::from_rgb(0xf0, 0xf0, 0xf0)),
    ));

    // draggable handles: x = freq, y = gain (high-pass: y ignored)
    for i in 0..BANDS {
        let mut band = params.band(i);
        let has_gain = KINDS[i] != FilterKind::HighPass;
        let center = pos2(
            x_of(rect, band.freq),
            y_of(rect, if has_gain { band.gain } else { 0.0 }),
        );
        let id = ui.id().with(("eq_handle", i));
        let hit = Rect::from_center_size(center, vec2(18.0, 18.0));
        let resp = ui.interact(hit, id, Sense::drag());
        if resp.dragged()
            && let Some(p) = resp.interact_pointer_pos()
        {
            band.freq = f_of(rect, p.x).round();
            if has_gain {
                band.gain = (db_of(rect, p.y) * 10.0).round() / 10.0;
            }
            params.set_band(i, band);
            changed = true;
        }
        // scroll over a handle changes q
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                band.q = (band.q * (1.0 + scroll * 0.002)).clamp(0.1, 10.0);
                params.set_band(i, band);
                changed = true;
            }
        }
        let color = if band.on {
            BAND_COLORS[i]
        } else {
            Color32::from_gray(90)
        };
        let radius = if resp.hovered() || resp.dragged() {
            8.0
        } else {
            6.0
        };
        painter.circle(center, radius, color, Stroke::new(1.5, Color32::WHITE));
        resp.on_hover_text(format!("{}: drag to move, scroll for Q", NAMES[i]));
    }

    ui.add_space(8.0);
    egui::Grid::new("eq_bands")
        .num_columns(5)
        .spacing([12.0, 6.0])
        .striped(true)
        .show(ui, |ui| {
            ui.label("");
            ui.label("band");
            ui.label("freq");
            ui.label("gain");
            ui.label("Q");
            ui.end_row();
            for i in 0..BANDS {
                let mut b = params.band(i);
                let before = b;
                ui.checkbox(&mut b.on, "");
                ui.colored_label(BAND_COLORS[i], NAMES[i]);
                ui.add(
                    egui::Slider::new(&mut b.freq, F_MIN..=F_MAX)
                        .logarithmic(true)
                        .suffix(" Hz")
                        .max_decimals(0),
                );
                ui.add_enabled(
                    KINDS[i] != FilterKind::HighPass,
                    egui::Slider::new(&mut b.gain, -DB_RANGE..=DB_RANGE)
                        .suffix(" dB")
                        .max_decimals(1),
                );
                ui.add(
                    egui::Slider::new(&mut b.q, 0.1..=10.0)
                        .logarithmic(true)
                        .max_decimals(2),
                );
                ui.end_row();
                if b != before {
                    params.set_band(i, b);
                    changed = true;
                }
            }
        });
    if ui.button("Reset EQ").clicked() {
        for (i, b) in crate::dsp::eq::DEFAULT_BANDS.iter().enumerate() {
            params.set_band(i, *b);
        }
        changed = true;
    }
    changed
}
