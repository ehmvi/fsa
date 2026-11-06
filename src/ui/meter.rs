//! vertical peak meter with decay and peak hold.

use std::time::Instant;

use egui::{Color32, Mesh, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};

use super::theme::{self, ALERT, DEEP, GRID, INK, MUTED, SIGNAL, SIGNAL_DIM};

use crate::dsp::lin_to_db;

const DB_MIN: f32 = -60.0;
/// fall rate of the bar
const DECAY_DB_PER_S: f32 = 24.0;
const HOLD_S: f32 = 1.5;
const WIDTH: f32 = 16.0;
const TICKS: [f32; 6] = [0.0, -6.0, -12.0, -24.0, -36.0, -48.0];

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

fn y_of(rect: Rect, db: f32) -> f32 {
    rect.bottom() - (db.clamp(DB_MIN, 0.0) - DB_MIN) / -DB_MIN * rect.height()
}

/// fill colour at a level: dim amber, amber from -18, red from -6
fn color_at(db: f32) -> Color32 {
    let mix = |a: Color32, b: Color32, t: f32| {
        let t = t.clamp(0.0, 1.0);
        let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
        Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
    };
    if db < -18.0 {
        mix(SIGNAL_DIM, SIGNAL, (db + 48.0) / 30.0)
    } else if db < -6.0 {
        SIGNAL
    } else {
        mix(SIGNAL, ALERT, (db + 6.0) / 3.0)
    }
}

/// dB labels lined up with the bars of `Meter::show`
pub fn scale(ui: &mut Ui, height: f32) {
    ui.vertical(|ui| {
        super::caption(ui, " ");
        let (rect, _) = ui.allocate_exact_size(vec2(22.0, height), Sense::hover());
        let painter = ui.painter();
        for db in TICKS.into_iter().chain([DB_MIN]) {
            painter.text(
                pos2(rect.right(), y_of(rect.shrink2(vec2(0.0, 3.0)), db)),
                egui::Align2::RIGHT_CENTER,
                format!("{db}"),
                theme::mono(10.0),
                theme::SUBTLE,
            );
        }
    });
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
        let size = vec2(WIDTH + 10.0, height + 44.0);
        let layout = egui::Layout::top_down(egui::Align::Center);
        ui.allocate_ui_with_layout(size, layout, |ui| {
            super::caption(ui, label);
            let (outer, _) = ui.allocate_exact_size(vec2(WIDTH, height), Sense::hover());
            let painter = ui.painter_at(outer.expand(1.0));
            painter.rect(
                outer,
                WIDTH / 2.0,
                DEEP,
                Stroke::new(1.0, theme::BORDER),
                egui::StrokeKind::Inside,
            );
            let rect = outer.shrink(3.0);
            for db in TICKS {
                let ty = y_of(rect, db);
                painter.line_segment(
                    [pos2(rect.left() + 1.0, ty), pos2(rect.right() - 1.0, ty)],
                    Stroke::new(1.0, GRID),
                );
            }

            // gradient fill, one quad per colour stop
            let level = self.level_db;
            if level > DB_MIN + 0.5 {
                let mut mesh = Mesh::default();
                let stops = [DB_MIN, -48.0, -18.0, -6.0, -3.0, 0.0];
                let mut prev = (DB_MIN, color_at(DB_MIN));
                for &db in &stops[1..] {
                    let top = db.min(level);
                    if top <= prev.0 {
                        break;
                    }
                    let c = color_at(top);
                    let v = mesh.vertices.len() as u32;
                    let (yb, yt) = (y_of(rect, prev.0), y_of(rect, top));
                    mesh.colored_vertex(pos2(rect.left(), yb), prev.1);
                    mesh.colored_vertex(pos2(rect.right(), yb), prev.1);
                    mesh.colored_vertex(pos2(rect.right(), yt), c);
                    mesh.colored_vertex(pos2(rect.left(), yt), c);
                    mesh.add_triangle(v, v + 1, v + 2);
                    mesh.add_triangle(v, v + 2, v + 3);
                    prev = (top, c);
                }
                painter.add(egui::Shape::mesh(mesh));
            }
            if self.hold_db > DB_MIN {
                let hy = y_of(rect, self.hold_db);
                painter.line_segment(
                    [pos2(rect.left(), hy), pos2(rect.right(), hy)],
                    Stroke::new(2.0, if self.hold_db > -3.0 { ALERT } else { INK }),
                );
            }
            let (text, color) = if self.hold_db <= DB_MIN {
                ("-∞".to_string(), theme::SUBTLE)
            } else {
                (
                    format!("{:.0}", self.hold_db),
                    if self.hold_db > -3.0 { ALERT } else { MUTED },
                )
            };
            ui.add_space(2.0);
            ui.label(RichText::new(text).font(theme::mono(10.5)).color(color));
        });
    }
}
