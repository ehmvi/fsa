//! rnnoise panel.

use egui::{ProgressBar, RichText, Ui};

use super::theme::{FAINT, SIGNAL};
use crate::dsp::denoise::DenoiseParams;

pub fn show(ui: &mut Ui, params: &DenoiseParams) {
    let mut s = params.get();
    let before = s;
    ui.label(
        RichText::new("RNNoise neural noise suppression. Adds a fixed 20 ms of latency.").weak(),
    );
    ui.add_space(6.0);
    let vad = params.vad.get();
    let open = s.vad_threshold <= 0.0 || vad >= s.vad_threshold;
    egui::Grid::new("denoise")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label("voice");
            let color = if open { SIGNAL } else { FAINT };
            ui.add(
                ProgressBar::new(vad)
                    .desired_width(220.0)
                    .fill(color)
                    .text(format!("{:.0} %", vad * 100.0)),
            )
            .on_hover_text("rnnoise's estimate that the current frame contains speech");
            ui.end_row();
            ui.label("strength");
            let mut pct = s.strength * 100.0;
            ui.add(
                egui::Slider::new(&mut pct, 0.0..=100.0)
                    .suffix(" %")
                    .max_decimals(0),
            )
            .on_hover_text("blend between the original and the denoised signal");
            s.strength = pct / 100.0;
            ui.end_row();
            ui.label("voice gate");
            let mut pct = s.vad_threshold * 100.0;
            ui.add(
                egui::Slider::new(&mut pct, 0.0..=99.0)
                    .max_decimals(0)
                    .custom_formatter(|v, _| {
                        if v <= 0.0 {
                            "off".into()
                        } else {
                            format!("{v:.0} %")
                        }
                    }),
            )
            .on_hover_text("mute the output while the voice probability is below this (0 = off)");
            s.vad_threshold = pct / 100.0;
            ui.end_row();
        });
    if s != before {
        params.set(&s);
    }
}
