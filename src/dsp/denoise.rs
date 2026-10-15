//! rnnoise-based noise suppression (via nnnoiseless) with an optional
//! voice-activity gate.
//!
//! rnnoise works on fixed 480-sample (10 ms) frames, so this unit buffers
//! one frame: constant 10 ms latency, the dry signal is delayed to match.
//!
//! real-time note: nnnoiseless keeps its fft plans and scratch buffers in
//! thread-local caches, so the very first frame processed on the audio
//! thread allocates them once. every later frame is allocation- and
//! lock-free.

use std::sync::Arc;

use nnnoiseless::DenoiseState;

use super::AtomicF32;

pub const FRAME: usize = DenoiseState::FRAME_SIZE;
/// rnnoise expects 16-bit pcm magnitudes in f32
const SCALE: f32 = 32768.0;
/// frames the vad gate stays open after voice stops (200 ms)
const HOLD_FRAMES: u32 = 20;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DenoiseSettings {
    /// 0 = dry, 1 = fully denoised
    pub strength: f32,
    /// voice probability below which the output is muted; 0 = gate off
    pub vad_threshold: f32,
}

impl Default for DenoiseSettings {
    fn default() -> Self {
        Self {
            strength: 1.0,
            vad_threshold: 0.0,
        }
    }
}

#[derive(Debug, Default)]
pub struct DenoiseParams {
    pub strength: AtomicF32,
    pub vad_threshold: AtomicF32,
    /// latest voice probability, written by the audio thread for the gui
    pub vad: AtomicF32,
}

impl DenoiseParams {
    pub fn new(s: &DenoiseSettings) -> Self {
        let p = Self::default();
        p.set(s);
        p
    }

    pub fn get(&self) -> DenoiseSettings {
        DenoiseSettings {
            strength: self.strength.get(),
            vad_threshold: self.vad_threshold.get(),
        }
    }

    pub fn set(&self, s: &DenoiseSettings) {
        self.strength.set(s.strength.clamp(0.0, 1.0));
        self.vad_threshold.set(s.vad_threshold.clamp(0.0, 1.0));
    }
}

pub struct Denoise {
    params: Arc<DenoiseParams>,
    state: Box<DenoiseState<'static>>,
    /// input collected for the next frame (also the delayed dry signal)
    input: [f32; FRAME],
    /// output of the previous frame, played back while the next one fills
    output: [f32; FRAME],
    scaled: [f32; FRAME],
    wet: [f32; FRAME],
    pos: usize,
    gate: f32,
    hold: u32,
}

impl Denoise {
    pub fn new(params: Arc<DenoiseParams>) -> Self {
        Self {
            params,
            state: DenoiseState::new(),
            input: [0.0; FRAME],
            output: [0.0; FRAME],
            scaled: [0.0; FRAME],
            wet: [0.0; FRAME],
            pos: 0,
            gate: 1.0,
            hold: 0,
        }
    }

    pub fn reset(&mut self) {
        self.input = [0.0; FRAME];
        self.output = [0.0; FRAME];
        self.pos = 0;
        self.gate = 1.0;
        self.hold = 0;
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        for s in buf.iter_mut() {
            let x = *s;
            *s = self.output[self.pos];
            self.input[self.pos] = x;
            self.pos += 1;
            if self.pos == FRAME {
                self.pos = 0;
                self.process_frame();
            }
        }
    }

    fn process_frame(&mut self) {
        for (d, s) in self.scaled.iter_mut().zip(&self.input) {
            *d = s * SCALE;
        }
        let vad = self.state.process_frame(&mut self.wet, &self.scaled);
        self.params.vad.set(vad);

        let s = self.params.get();
        let target = if s.vad_threshold <= 0.0 || vad >= s.vad_threshold {
            self.hold = HOLD_FRAMES;
            1.0
        } else if self.hold > 0 {
            self.hold -= 1;
            1.0
        } else {
            0.0
        };
        // ramp the gate linearly across the frame to avoid clicks
        let step = (target - self.gate) / FRAME as f32;
        for i in 0..FRAME {
            let dry = self.input[i];
            let wet = self.wet[i] / SCALE;
            self.gate += step;
            self.output[i] = (dry + (wet - dry) * s.strength) * self.gate;
        }
        self.gate = target;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(seed: &mut u32) -> f32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;
        *seed as f32 / u32::MAX as f32 - 0.5
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn latency_is_one_frame_when_dry() {
        let params = Arc::new(DenoiseParams::new(&DenoiseSettings {
            strength: 0.0,
            vad_threshold: 0.0,
        }));
        let mut d = Denoise::new(params);
        let mut buf: Vec<f32> = (0..FRAME * 3).map(|i| (i as f32 * 0.01).sin()).collect();
        let orig = buf.clone();
        // odd block sizes, like pipewire quanta
        for chunk in buf.chunks_mut(256) {
            d.process(chunk);
        }
        assert!(buf[..FRAME].iter().all(|&s| s == 0.0));
        for i in FRAME..FRAME * 3 {
            assert!((buf[i] - orig[i - FRAME]).abs() < 1e-6);
        }
    }

    #[test]
    fn suppresses_stationary_noise() {
        // low-passed noise, roughly what a fan or air conditioner sounds like
        let params = Arc::new(DenoiseParams::new(&DenoiseSettings::default()));
        let mut d = Denoise::new(params);
        let (mut seed, mut y) = (7u32, 0.0f32);
        let mut buf: Vec<f32> = (0..48_000 * 2)
            .map(|_| {
                y += 0.05 * (noise(&mut seed) - y);
                y * 0.2
            })
            .collect();
        let before = rms(&buf[48_000..]);
        d.process(&mut buf);
        let after = rms(&buf[48_000..]);
        assert!(after < before * 0.1, "rms {before} -> {after}");
    }

    #[test]
    fn keeps_tonal_signal() {
        let params = Arc::new(DenoiseParams::new(&DenoiseSettings::default()));
        let mut d = Denoise::new(params);
        let mut buf: Vec<f32> = (0..48_000 * 2)
            .map(|i| 0.2 * (i as f32 * std::f32::consts::TAU * 220.0 / 48_000.0).sin())
            .collect();
        let before = rms(&buf[48_000..]);
        d.process(&mut buf);
        assert!((rms(&buf[48_000..]) / before - 1.0).abs() < 0.1);
    }

    #[test]
    fn vad_gate_mutes_noise() {
        let s = DenoiseSettings {
            strength: 0.0,
            vad_threshold: 0.9,
        };
        let params = Arc::new(DenoiseParams::new(&s));
        let mut d = Denoise::new(params);
        let mut seed = 1;
        let mut buf: Vec<f32> = (0..48_000).map(|_| noise(&mut seed) * 0.05).collect();
        d.process(&mut buf);
        assert!(rms(&buf[36_000..]) < 1e-4);
    }
}
