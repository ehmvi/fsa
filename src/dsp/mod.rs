//! built-in dsp. all `process` functions are real-time safe.

pub mod biquad;
pub mod eq;
pub mod gate;

use std::sync::atomic::{AtomicU32, Ordering};

/// f32 stored in an atomic, for passing parameters to the audio thread
#[derive(Debug, Default)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn new(v: f32) -> Self {
        Self(AtomicU32::new(v.to_bits()))
    }

    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }

    /// raises the stored value to `v` (for peak meters). load/store is not
    /// one atomic step, but a lost peak between gui reads is harmless.
    pub fn raise(&self, v: f32) {
        if v > self.get() {
            self.set(v);
        }
    }

    /// reads and resets to zero
    pub fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Ordering::Relaxed))
    }
}

pub fn peak(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

pub fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

pub fn lin_to_db(lin: f32) -> f32 {
    20.0 * lin.max(1e-9).log10()
}

/// linear gain ramp used for click-free on/off and global bypass
#[derive(Debug, Clone)]
pub struct Fade {
    pub gain: f32,
    target: f32,
    step: f32,
}

impl Fade {
    /// ramp time for a full 0..1 transition
    pub const MS: f32 = 10.0;

    pub fn new(on: bool, sample_rate: f32) -> Self {
        let v = if on { 1.0 } else { 0.0 };
        Self {
            gain: v,
            target: v,
            step: 1.0 / (Self::MS * 0.001 * sample_rate),
        }
    }

    pub fn set(&mut self, on: bool) {
        self.target = if on { 1.0 } else { 0.0 };
    }

    /// fully off and staying off: processing can be skipped
    pub fn is_silent(&self) -> bool {
        self.gain == 0.0 && self.target == 0.0
    }

    /// fully on and staying on: no mixing needed
    pub fn is_full(&self) -> bool {
        self.gain == 1.0 && self.target == 1.0
    }

    /// `out = dry + (wet - dry) * gain`, ramping gain toward the target.
    /// the result is written into `wet`.
    pub fn mix(&mut self, dry: &[f32], wet: &mut [f32]) {
        if self.is_full() {
            return;
        }
        for (w, &d) in wet.iter_mut().zip(dry) {
            if self.gain < self.target {
                self.gain = (self.gain + self.step).min(self.target);
            } else if self.gain > self.target {
                self.gain = (self.gain - self.step).max(self.target);
            }
            *w = d + (*w - d) * self.gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fade_ramps_without_jumps() {
        let mut f = Fade::new(false, 48_000.0);
        f.set(true);
        let dry = [0.0f32; 1000];
        let mut wet = [1.0f32; 1000];
        f.mix(&dry, &mut wet);
        let max_step = wet
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(max_step < 0.003, "{max_step}");
        assert!(f.is_full());
    }
}
