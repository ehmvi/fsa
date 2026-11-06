//! noise gate panel.

use std::sync::atomic::Ordering;

use egui::{RichText, Ui};

use super::knob::Knob;
use super::theme::MUTED;
use super::{PANEL_KNOB, knob_columns, panel_footer, panel_knob_well, panel_well, status_strip};
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
    ui.add_space(20.0);
    panel_well().show(ui, |ui| {
        ui.set_width(ui.available_width());
        let t = |db: f32| (db - FLOOR_DB) / -FLOOR_DB;
        let readout = if level <= FLOOR_DB {
            "-∞ dB".to_string()
        } else {
            format!("{level:.0} dB")
        };
        status_strip(
            ui,
            if open { "OPEN" } else { "CLOSED" },
            open,
            (t(level), Some(t(s.threshold_db)), open),
            &readout,
            &[
                (0.0, "-80"),
                (0.25, "-60"),
                (0.5, "-40"),
                (0.75, "-20"),
                (1.0, "0 dB"),
            ],
        )
        .on_hover_text("the detector level, with the threshold marked");
    });
    ui.add_space(28.0);
    panel_knob_well(ui, |ui| {
        let db = |v: f32| format!("{v:.1} dB");
        let ms = |v: f32| {
            if v < 10.0 {
                format!("{v:.1} ms")
            } else {
                format!("{v:.0} ms")
            }
        };
        knob_columns(ui, 5, 5, |ui, i| {
            let knob = match i {
                0 => Knob::new(&mut s.threshold_db, -80.0..=0.0, "THRESHOLD", &db).default(-50.0),
                1 => Knob::new(&mut s.range_db, -80.0..=0.0, "RANGE", &db).default(-40.0),
                2 => Knob::new(&mut s.attack_ms, 0.1..=50.0, "ATTACK", &ms)
                    .log()
                    .default(2.0),
                3 => Knob::new(&mut s.hold_ms, 0.0..=500.0, "HOLD", &ms).default(80.0),
                _ => Knob::new(&mut s.release_ms, 5.0..=1000.0, "RELEASE", &ms)
                    .log()
                    .default(150.0),
            };
            let resp = ui.add(knob.radius(PANEL_KNOB));
            match i {
                0 => {
                    resp.on_hover_text("the gate opens above this level");
                }
                1 => {
                    resp.on_hover_text("attenuation while the gate is closed");
                }
                _ => {}
            }
        });
    });
    panel_footer(
        ui,
        "Drag or scroll a knob, hold shift for fine steps, double-click to reset.",
    );
    if s != before {
        params.set(&s);
        return true;
    }
    false
}
