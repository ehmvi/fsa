//! 5-band parametric eq: high-pass, low shelf, two peaks, high shelf.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::AtomicF32;
use super::biquad::{Biquad, Coeffs, FilterKind};

pub const BANDS: usize = 5;
pub const KINDS: [FilterKind; BANDS] = [
    FilterKind::HighPass,
    FilterKind::LowShelf,
    FilterKind::Peak,
    FilterKind::Peak,
    FilterKind::HighShelf,
];
pub const NAMES: [&str; BANDS] = ["High-pass", "Low shelf", "Peak 1", "Peak 2", "High shelf"];

#[derive(Debug)]
pub struct BandParams {
    pub on: AtomicBool,
    pub freq: AtomicF32,
    pub gain: AtomicF32,
    pub q: AtomicF32,
}

/// plain-data snapshot of one band, used for config and coefficient math
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Band {
    pub on: bool,
    pub freq: f32,
    pub gain: f32,
    pub q: f32,
}

impl Band {
    pub fn coeffs(&self, kind: FilterKind, sample_rate: f32) -> Coeffs {
        if !self.on {
            return Coeffs::IDENTITY;
        }
        let gain = if kind == FilterKind::HighPass {
            0.0
        } else {
            self.gain
        };
        Coeffs::new(kind, sample_rate, self.freq, gain, self.q)
    }
}

pub const DEFAULT_BANDS: [Band; BANDS] = [
    Band {
        on: true,
        freq: 80.0,
        gain: 0.0,
        q: 0.707,
    },
    Band {
        on: true,
        freq: 150.0,
        gain: 0.0,
        q: 0.707,
    },
    Band {
        on: true,
        freq: 400.0,
        gain: 0.0,
        q: 1.0,
    },
    Band {
        on: true,
        freq: 3000.0,
        gain: 0.0,
        q: 1.0,
    },
    Band {
        on: true,
        freq: 8000.0,
        gain: 0.0,
        q: 0.707,
    },
];

#[derive(Debug)]
pub struct EqParams {
    pub bands: [BandParams; BANDS],
}

impl EqParams {
    pub fn new(bands: &[Band; BANDS]) -> Self {
        Self {
            bands: bands.map(|b| BandParams {
                on: AtomicBool::new(b.on),
                freq: AtomicF32::new(b.freq),
                gain: AtomicF32::new(b.gain),
                q: AtomicF32::new(b.q),
            }),
        }
    }

    pub fn band(&self, i: usize) -> Band {
        let p = &self.bands[i];
        Band {
            on: p.on.load(Ordering::Relaxed),
            freq: p.freq.get(),
            gain: p.gain.get(),
            q: p.q.get(),
        }
    }

    pub fn set_band(&self, i: usize, b: Band) {
        let p = &self.bands[i];
        p.on.store(b.on, Ordering::Relaxed);
        p.freq.set(b.freq);
        p.gain.set(b.gain);
        p.q.set(b.q);
    }

    pub fn snapshot(&self) -> [Band; BANDS] {
        std::array::from_fn(|i| self.band(i))
    }

    /// combined magnitude response in db, for drawing
    pub fn response_db(&self, sample_rate: f32, freq: f32) -> f32 {
        (0..BANDS)
            .map(|i| {
                self.band(i)
                    .coeffs(KINDS[i], sample_rate)
                    .magnitude_db(sample_rate, freq)
            })
            .sum()
    }
}

pub struct Eq {
    params: Arc<EqParams>,
    sample_rate: f32,
    current: [Option<Band>; BANDS],
    filters: [Biquad; BANDS],
}

impl Eq {
    pub fn new(params: Arc<EqParams>, sample_rate: f32) -> Self {
        Self {
            params,
            sample_rate,
            current: [None; BANDS],
            filters: Default::default(),
        }
    }

    pub fn reset(&mut self) {
        for f in &mut self.filters {
            f.reset();
        }
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        for i in 0..BANDS {
            let band = self.params.band(i);
            if self.current[i] != Some(band) {
                self.filters[i].coeffs = band.coeffs(KINDS[i], self.sample_rate);
                self.current[i] = Some(band);
            }
            if band.on {
                self.filters[i].process(buf);
            }
        }
    }
}
