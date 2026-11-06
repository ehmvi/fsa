//! rnnoise panel.

use egui::{RichText, Ui};

use super::knob::Knob;
use super::theme::{self, MUTED};
use crate::dsp::denoise::DenoiseParams;

pub fn show(ui: &mut Ui, params: &DenoiseParams) {
    let mut s = params.get();
    let before = s;
    ui.label(
        RichText::new("RNNoise neural noise suppression. Adds a fixed 20 ms of latency.")
            .color(MUTED),
    );
    ui.add_space(12.0);
    let vad = params.vad.get();
    let open = s.vad_threshold <= 0.0 || vad >= s.vad_threshold;
    theme::well().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            super::state_chip(ui, "VOICE", vad >= s.vad_threshold.max(0.5))
                .on_hover_text("rnnoise's estimate that the current frame contains speech");
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.add_space(7.0);
                let w = (ui.available_width() - 70.0).max(120.0);
                let marker = (s.vad_threshold > 0.0).then_some(s.vad_threshold);
                let rect = super::level_bar(ui, w, vad, marker, open);
                ui.painter().text(
                    rect.right_center() + egui::vec2(10.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    format!("{:.0} %", vad * 100.0),
                    theme::mono(11.0),
                    MUTED,
                );
            });
        });
    });
    ui.add_space(12.0);
    theme::well().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 28.0;
            let mut pct = s.strength * 100.0;
            let fmt = |v: f32| format!("{v:.0} %");
            ui.add(
                Knob::new(&mut pct, 0.0..=100.0, "STRENGTH", &fmt)
                    .default(100.0)
                    .radius(24.0),
            )
            .on_hover_text("blend between the original and the denoised signal");
            s.strength = pct / 100.0;
            let mut pct = s.vad_threshold * 100.0;
            let fmt = |v: f32| {
                if v < 0.5 {
                    "off".into()
                } else {
                    format!("{v:.0} %")
                }
            };
            ui.add(
                Knob::new(&mut pct, 0.0..=99.0, "VOICE GATE", &fmt)
                    .default(0.0)
                    .radius(24.0),
            )
            .on_hover_text("mute the output while the voice probability is below this (0 = off)");
            s.vad_threshold = if pct < 0.5 { 0.0 } else { pct / 100.0 };
        });
    });
    if s != before {
        params.set(&s);
    }
}
