//! simple noise gate with hysteresis, hold and smoothed gain.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{AtomicF32, db_to_lin};

/// the gate closes this many db below the open threshold
const HYSTERESIS_DB: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GateSettings {
    pub threshold_db: f32,
    pub attack_ms: f32,
    pub hold_ms: f32,
    pub release_ms: f32,
    /// attenuation when closed, e.g. -40 db
    pub range_db: f32,
}

impl Default for GateSettings {
    fn default() -> Self {
        Self {
            threshold_db: -50.0,
            attack_ms: 2.0,
            hold_ms: 80.0,
            release_ms: 150.0,
            range_db: -40.0,
        }
    }
}

#[derive(Debug, Default)]
pub struct GateParams {
    pub threshold_db: AtomicF32,
    pub attack_ms: AtomicF32,
    pub hold_ms: AtomicF32,
    pub release_ms: AtomicF32,
    pub range_db: AtomicF32,
    /// written by the audio thread, for the gui indicator
    pub open: AtomicBool,
}

impl GateParams {
    pub fn new(s: &GateSettings) -> Self {
        let p = Self::default();
        p.set(s);
        p
    }

    pub fn get(&self) -> GateSettings {
        GateSettings {
            threshold_db: self.threshold_db.get(),
            attack_ms: self.attack_ms.get(),
            hold_ms: self.hold_ms.get(),
            release_ms: self.release_ms.get(),
            range_db: self.range_db.get(),
        }
    }

    pub fn set(&self, s: &GateSettings) {
        self.threshold_db.set(s.threshold_db);
        self.attack_ms.set(s.attack_ms);
        self.hold_ms.set(s.hold_ms);
        self.release_ms.set(s.release_ms);
        self.range_db.set(s.range_db);
    }
}

pub struct Gate {
    params: Arc<GateParams>,
    sample_rate: f32,
    current: Option<GateSettings>,
    open_lin: f32,
    close_lin: f32,
    floor: f32,
    attack: f32,
    release: f32,
    env_decay: f32,
    hold_samples: u32,
    env: f32,
    hold: u32,
    open: bool,
    gain: f32,
}

impl Gate {
    pub fn new(params: Arc<GateParams>, sample_rate: f32) -> Self {
        let mut g = Self {
            params,
            sample_rate,
            current: None,
            open_lin: 0.0,
            close_lin: 0.0,
            floor: 0.0,
            attack: 0.0,
            release: 0.0,
            env_decay: (-1.0 / (0.010 * sample_rate)).exp(),
            hold_samples: 0,
            env: 0.0,
            hold: 0,
            open: false,
            gain: 0.0,
        };
        g.update();
        g.gain = g.floor;
        g
    }

    pub fn reset(&mut self) {
        self.env = 0.0;
        self.hold = 0;
        self.open = false;
        self.gain = self.floor;
    }

    fn coef(&self, ms: f32) -> f32 {
        1.0 - (-1.0 / (ms.max(0.1) * 0.001 * self.sample_rate)).exp()
    }

    fn update(&mut self) {
        let s = self.params.get();
        if self.current == Some(s) {
            return;
        }
        self.open_lin = db_to_lin(s.threshold_db);
        self.close_lin = db_to_lin(s.threshold_db - HYSTERESIS_DB);
        self.floor = db_to_lin(s.range_db.min(0.0));
        self.attack = self.coef(s.attack_ms);
        self.release = self.coef(s.release_ms);
        self.hold_samples = (s.hold_ms.max(0.0) * 0.001 * self.sample_rate) as u32;
        self.current = Some(s);
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        self.update();
        for s in buf.iter_mut() {
            // peak envelope with ~10 ms decay
            self.env = s.abs().max(self.env * self.env_decay);
            if self.env >= self.open_lin {
                self.open = true;
                self.hold = self.hold_samples;
            } else if self.env < self.close_lin {
                if self.hold > 0 {
                    self.hold -= 1;
                } else {
                    self.open = false;
                }
            }
            let (target, coef) = if self.open {
                (1.0, self.attack)
            } else {
                (self.floor, self.release)
            };
            self.gain += (target - self.gain) * coef;
            *s *= self.gain;
        }
        self.params.open.store(self.open, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closes_on_noise_opens_on_signal() {
        let params = Arc::new(GateParams::new(&GateSettings::default()));
        let mut g = Gate::new(params.clone(), 48_000.0);
        let mut quiet = vec![0.0005f32; 48_000];
        g.process(&mut quiet);
        assert!(!params.open.load(Ordering::Relaxed));
        assert!(quiet[47_999].abs() < 0.0005 * 0.02);

        let mut loud: Vec<f32> = (0..4800).map(|n| 0.3 * (n as f32 * 0.1).sin()).collect();
        g.process(&mut loud);
        assert!(params.open.load(Ordering::Relaxed));
        let tail = loud[4000..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(tail > 0.29);
    }
}
