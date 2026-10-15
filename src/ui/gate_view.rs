//! noise gate panel.

use std::sync::atomic::Ordering;

use egui::{RichText, Ui};

use super::{GREEN, GREY};
use crate::dsp::gate::GateParams;

/// returns true if any parameter changed
pub fn show(ui: &mut Ui, params: &GateParams) -> bool {
    let mut s = params.get();
    let before = s;
    let open = params.open.load(Ordering::Relaxed);
    ui.horizontal(|ui| {
        ui.label("state:");
        let (t, c) = if open {
            ("OPEN", GREEN)
        } else {
            ("CLOSED", GREY)
        };
        ui.label(RichText::new(t).strong().color(c));
    });
    ui.add_space(6.0);
    egui::Grid::new("gate")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label("threshold");
            ui.add(
                egui::Slider::new(&mut s.threshold_db, -80.0..=0.0)
                    .suffix(" dB")
                    .max_decimals(1),
            );
            ui.end_row();
            ui.label("range");
            ui.add(
                egui::Slider::new(&mut s.range_db, -80.0..=0.0)
                    .suffix(" dB")
                    .max_decimals(1),
            )
            .on_hover_text("attenuation while the gate is closed");
            ui.end_row();
            ui.label("attack");
            ui.add(
                egui::Slider::new(&mut s.attack_ms, 0.1..=50.0)
                    .logarithmic(true)
                    .suffix(" ms"),
            );
            ui.end_row();
            ui.label("hold");
            ui.add(
                egui::Slider::new(&mut s.hold_ms, 0.0..=500.0)
                    .suffix(" ms")
                    .max_decimals(0),
            );
            ui.end_row();
            ui.label("release");
            ui.add(
                egui::Slider::new(&mut s.release_ms, 5.0..=1000.0)
                    .logarithmic(true)
                    .suffix(" ms"),
            );
            ui.end_row();
        });
    if s != before {
        params.set(&s);
        return true;
    }
    false
}
