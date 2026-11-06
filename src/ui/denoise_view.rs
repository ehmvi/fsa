//! rnnoise panel.

use egui::{RichText, Ui};

use super::knob::Knob;
use super::theme::{self, MUTED};
use super::{PANEL_KNOB, knob_cell, knob_pitch, panel_footer, panel_well, status_strip};
use crate::dsp::denoise::DenoiseParams;

pub fn show(ui: &mut Ui, params: &DenoiseParams) {
    let mut s = params.get();
    let before = s;
    ui.label(
        RichText::new("RNNoise neural noise suppression. Adds a fixed 20 ms of latency.")
            .color(MUTED),
    );
    ui.add_space(20.0);
    let vad = params.vad.get();
    let open = s.vad_threshold <= 0.0 || vad >= s.vad_threshold;
    let voice = vad >= s.vad_threshold.max(0.5);
    panel_well().show(ui, |ui| {
        ui.set_width(ui.available_width());
        let marker = (s.vad_threshold > 0.0).then_some(s.vad_threshold);
        status_strip(
            ui,
            if voice { "VOICE" } else { "NO VOICE" },
            voice,
            (vad, marker, open),
            &format!("{:.0} %", vad * 100.0),
            &[(0.0, "0"), (0.5, "50"), (1.0, "100 %")],
        )
        .on_hover_text("rnnoise's estimate that the current frame contains speech");
    });
    ui.add_space(20.0);
    panel_well().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            // same grid as the gate's knobs, so the panels line up
            let pitch = knob_pitch(ui, 5);
            let mut strength = s.strength * 100.0;
            let mut gate = s.vad_threshold * 100.0;
            let pct = |v: f32| format!("{v:.0} %");
            let gate_fmt = |v: f32| {
                if v < 0.5 {
                    "off".into()
                } else {
                    format!("{v:.0} %")
                }
            };
            knob_cell(ui, pitch, PANEL_KNOB, |ui| {
                ui.add(
                    Knob::new(&mut strength, 0.0..=100.0, "STRENGTH", &pct)
                        .default(100.0)
                        .radius(PANEL_KNOB),
                )
                .on_hover_text("blend between the original and the denoised signal");
            });
            knob_cell(ui, pitch, PANEL_KNOB, |ui| {
                ui.add(
                    Knob::new(&mut gate, 0.0..=99.0, "VOICE GATE", &gate_fmt)
                        .default(0.0)
                        .radius(PANEL_KNOB),
                )
                .on_hover_text(
                    "mute the output while the voice probability is below this (0 = off)",
                );
            });
            s.strength = strength / 100.0;
            s.vad_threshold = if gate < 0.5 { 0.0 } else { gate / 100.0 };
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.add_space(24.0);
                ui.label(
                    RichText::new(
                        "Strength blends the cleaned signal with the original. \
                         Voice gate mutes the output while the chance of speech \
                         stays under its setting.",
                    )
                    .color(theme::SUBTLE),
                );
            });
        });
    });
    panel_footer(
        ui,
        "Drag or scroll a knob, hold shift for fine steps, double-click to reset.",
    );
    if s != before {
        params.set(&s);
    }
}
