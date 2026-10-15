//! vertical peak meter with decay and peak hold.

use std::time::Instant;

use egui::{Color32, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::dsp::lin_to_db;

const DB_MIN: f32 = -60.0;
/// fall rate of the bar
const DECAY_DB_PER_S: f32 = 24.0;
const HOLD_S: f32 = 1.5;

pub struct Meter {
    level_db: f32,
    hold_db: f32,
    hold_at: Instant,
    last: Instant,
}

impl Default for Meter {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            level_db: DB_MIN,
            hold_db: DB_MIN,
            hold_at: now,
            last: now,
        }
    }
}

impl Meter {
    /// feed the peak (linear) collected since the previous frame
    pub fn update(&mut self, peak: f32) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32();
        self.last = now;
        let db = lin_to_db(peak).max(DB_MIN);
        self.level_db = db.max(self.level_db - DECAY_DB_PER_S * dt);
        if db >= self.hold_db || now.duration_since(self.hold_at).as_secs_f32() > HOLD_S {
            self.hold_db = db;
            self.hold_at = now;
        }
    }

    pub fn show(&self, ui: &mut Ui, label: &str, height: f32) {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(label).small().strong());
            let (rect, _) = ui.allocate_exact_size(vec2(22.0, height), Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 2.0, Color32::from_gray(25));
            let y = |db: f32| rect.bottom() - (db - DB_MIN) / -DB_MIN * rect.height();

            // coloured segments: green up to -18, yellow up to -6, red above
            let level = self.level_db;
            for (lo, hi, color) in [
                (DB_MIN, -18.0, Color32::from_rgb(0x2e, 0xa0, 0x43)),
                (-18.0, -6.0, Color32::from_rgb(0xd9, 0xc2, 0x1f)),
                (-6.0, 0.0, Color32::from_rgb(0xd9, 0x3b, 0x2f)),
            ] {
                if level > lo {
                    let top = y(level.min(hi));
                    painter.rect_filled(
                        Rect::from_min_max(
                            pos2(rect.left() + 2.0, top),
                            pos2(rect.right() - 2.0, y(lo)),
                        ),
                        0.0,
                        color,
                    );
                }
            }
            if self.hold_db > DB_MIN {
                let hy = y(self.hold_db.min(0.0));
                painter.line_segment(
                    [pos2(rect.left(), hy), pos2(rect.right(), hy)],
                    Stroke::new(2.0, Color32::WHITE),
                );
            }
            for db in [-48.0, -36.0, -24.0, -12.0, -6.0] {
                let ty = y(db);
                painter.line_segment(
                    [pos2(rect.left(), ty), pos2(rect.left() + 4.0, ty)],
                    Stroke::new(1.0, Color32::from_gray(140)),
                );
            }
            let text = if self.hold_db <= DB_MIN {
                "-inf".to_string()
            } else {
                format!("{:.0}", self.hold_db)
            };
            ui.label(egui::RichText::new(text).small().monospace());
        });
    }
}
