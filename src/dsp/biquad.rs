//! rbj audio eq cookbook biquads, transposed direct form ii in f64.

use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FilterKind {
    HighPass,
    LowShelf,
    Peak,
    HighShelf,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coeffs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Coeffs {
    pub const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    pub fn new(kind: FilterKind, sample_rate: f32, freq: f32, gain_db: f32, q: f32) -> Self {
        let fs = sample_rate as f64;
        let f0 = (freq as f64).clamp(10.0, fs * 0.49);
        let q = (q as f64).max(0.05);
        let a = 10f64.powf(gain_db as f64 / 40.0);
        let w0 = 2.0 * PI * f0 / fs;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);

        let (b0, b1, b2, a0, a1, a2) = match kind {
            FilterKind::HighPass => (
                (1.0 + cos) / 2.0,
                -(1.0 + cos),
                (1.0 + cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            FilterKind::Peak => (
                1.0 + alpha * a,
                -2.0 * cos,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos,
                1.0 - alpha / a,
            ),
            FilterKind::LowShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cos + s),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - s),
                    (a + 1.0) + (a - 1.0) * cos + s,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - s,
                )
            }
            FilterKind::HighShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cos + s),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - s),
                    (a + 1.0) - (a - 1.0) * cos + s,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - s,
                )
            }
        };
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// magnitude response in db at `freq`
    pub fn magnitude_db(&self, sample_rate: f32, freq: f32) -> f32 {
        let w = 2.0 * PI * freq as f64 / sample_rate as f64;
        let (s1, c1) = w.sin_cos();
        let (s2, c2) = (2.0 * w).sin_cos();
        let num_re = self.b0 + self.b1 * c1 + self.b2 * c2;
        let num_im = -(self.b1 * s1 + self.b2 * s2);
        let den_re = 1.0 + self.a1 * c1 + self.a2 * c2;
        let den_im = -(self.a1 * s1 + self.a2 * s2);
        let num = num_re * num_re + num_im * num_im;
        let den = den_re * den_re + den_im * den_im;
        (10.0 * (num / den).max(1e-20).log10()) as f32
    }
}

#[derive(Clone, Debug)]
pub struct Biquad {
    pub coeffs: Coeffs,
    z1: f64,
    z2: f64,
}

impl Default for Biquad {
    fn default() -> Self {
        Self {
            coeffs: Coeffs::IDENTITY,
            z1: 0.0,
            z2: 0.0,
        }
    }
}

impl Biquad {
    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let c = &self.coeffs;
        let x = x as f64;
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        y as f32
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        for s in buf {
            *s = self.tick(*s);
        }
        // flush denormals once per block
        if self.z1.abs() < 1e-30 {
            self.z1 = 0.0;
        }
        if self.z2.abs() < 1e-30 {
            self.z2 = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f32 = 48_000.0;

    #[test]
    fn peak_gain_at_center() {
        let c = Coeffs::new(FilterKind::Peak, FS, 1000.0, 6.0, 1.0);
        assert!((c.magnitude_db(FS, 1000.0) - 6.0).abs() < 0.01);
        assert!(c.magnitude_db(FS, 20.0).abs() < 0.1);
    }

    #[test]
    fn highpass_is_minus_3db_at_cutoff_for_butterworth() {
        let c = Coeffs::new(
            FilterKind::HighPass,
            FS,
            100.0,
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
        );
        assert!((c.magnitude_db(FS, 100.0) + 3.01).abs() < 0.05);
        assert!(c.magnitude_db(FS, 10_000.0).abs() < 0.01);
        assert!(c.magnitude_db(FS, 25.0) < -20.0);
    }

    #[test]
    fn shelves_reach_full_gain() {
        let lo = Coeffs::new(FilterKind::LowShelf, FS, 200.0, -4.0, 0.707);
        assert!((lo.magnitude_db(FS, 20.0) + 4.0).abs() < 0.1);
        let hi = Coeffs::new(FilterKind::HighShelf, FS, 6000.0, 3.0, 0.707);
        assert!((hi.magnitude_db(FS, 20_000.0) - 3.0).abs() < 0.2);
    }

    #[test]
    fn tick_matches_magnitude() {
        let mut bq = Biquad {
            coeffs: Coeffs::new(FilterKind::Peak, FS, 1000.0, 12.0, 2.0),
            ..Default::default()
        };
        let mut peak = 0.0f32;
        for n in 0..48_000 {
            let x = (2.0 * std::f32::consts::PI * 1000.0 * n as f32 / FS).sin();
            let y = bq.tick(x);
            if n > 24_000 {
                peak = peak.max(y.abs());
            }
        }
        assert!((20.0 * peak.log10() - 12.0).abs() < 0.1);
    }
}
