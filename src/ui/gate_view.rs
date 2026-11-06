//! noise gate panel.

use std::sync::atomic::Ordering;

use egui::{RichText, Ui};

use super::knob::Knob;
use super::theme::{self, FAINT, MUTED};
use crate::dsp::gate::GateParams;
use crate::dsp::lin_to_db;

const FLOOR_DB: f32 = -80.0;

/// returns true if any parameter changed
pub fn show(ui: &mut Ui, params: &GateParams) -> bool {
    let mut s = params.get();
    let before = s;
    let open = params.open.load(Ordering::Relaxed);

    // detector level with a falling bar, kept between frames
    let id = ui.id().with("gate_level");
    let peak = lin_to_db(params.level.take()).max(FLOOR_DB);
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    let level = ui.data_mut(|d| {
        let l = d.get_temp_mut_or(id, FLOOR_DB);
        *l = peak.max(*l - 30.0 * dt);
        *l
    });

    ui.label(
        RichText::new("Mutes the mic between words, when the level stays under the threshold.")
            .color(MUTED),
    );
    ui.add_space(12.0);
    theme::well().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            super::state_chip(ui, if open { "OPEN" } else { "CLOSED" }, open);
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.add_space(7.0);
                let w = (ui.available_width() - 70.0).max(120.0);
                let t = |db: f32| (db - FLOOR_DB) / -FLOOR_DB;
                let rect = super::level_bar(ui, w, t(level), Some(t(s.threshold_db)), open);
                ui.painter().text(
                    rect.right_center() + egui::vec2(10.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    format!("{level:.0} dB"),
                    theme::mono(11.0),
                    if level <= FLOOR_DB { FAINT } else { MUTED },
                );
            });
        });
    });
    ui.add_space(12.0);
    theme::well().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            // spread the knobs over the well, within reason
            let gap = ((ui.available_width() - 5.0 * 70.0) / 4.0).clamp(18.0, 64.0);
            ui.spacing_mut().item_spacing.x = gap;
            let db = |v: f32| format!("{v:.1} dB");
            let ms = |v: f32| {
                if v < 10.0 {
                    format!("{v:.1} ms")
                } else {
                    format!("{v:.0} ms")
                }
            };
            ui.add(
                Knob::new(&mut s.threshold_db, -80.0..=0.0, "THRESHOLD", &db)
                    .default(-50.0)
                    .radius(24.0),
            )
            .on_hover_text("the gate opens above this level");
            ui.add(
                Knob::new(&mut s.range_db, -80.0..=0.0, "RANGE", &db)
                    .default(-40.0)
                    .radius(24.0),
            )
            .on_hover_text("attenuation while the gate is closed");
            ui.add(
                Knob::new(&mut s.attack_ms, 0.1..=50.0, "ATTACK", &ms)
                    .log()
                    .default(2.0)
                    .radius(24.0),
            );
            ui.add(
                Knob::new(&mut s.hold_ms, 0.0..=500.0, "HOLD", &ms)
                    .default(80.0)
                    .radius(24.0),
            );
            ui.add(
                Knob::new(&mut s.release_ms, 5.0..=1000.0, "RELEASE", &ms)
                    .log()
                    .default(150.0)
                    .radius(24.0),
            );
        });
    });
    if s != before {
        params.set(&s);
        return true;
    }
    false
}
