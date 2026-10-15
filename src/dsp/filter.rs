//! eq filter design.
//!
//! every shape starts as an analog prototype and is turned into digital
//! second-order sections with magnitude matching (vicanek, "matched second
//! order digital filters", 2016): poles are mapped exactly and the
//! numerator is solved so the digital response equals the analog one at
//! dc, nyquist and the band frequency. unlike bilinear designs, bells and
//! shelves keep their analog shape up to nyquist instead of cramping.
//!
//! sections run as trapezoidal state variable filters (simper), which stay
//! clean while their coefficients move, so bands can be swept and dynamic
//! bands can modulate without zipper noise.

use std::f64::consts::PI;

use serde::{Deserialize, Serialize};

/// 96 db/oct needs 8 second-order sections
pub const MAX_SECTIONS: usize = 8;
/// available cut slopes in db/oct
pub const SLOPES: [u8; 9] = [6, 12, 18, 24, 30, 36, 48, 72, 96];
pub const FREQ_MIN: f32 = 10.0;
pub const FREQ_MAX: f32 = 22_000.0;
pub const GAIN_MAX: f32 = 30.0;
pub const Q_MIN: f32 = 0.025;
pub const Q_MAX: f32 = 40.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    #[default]
    Bell,
    LowShelf,
    HighShelf,
    TiltShelf,
    LowCut,
    HighCut,
    Notch,
    BandPass,
    AllPass,
}

impl Shape {
    pub const ALL: [Shape; 9] = [
        Shape::Bell,
        Shape::LowShelf,
        Shape::LowCut,
        Shape::HighShelf,
        Shape::HighCut,
        Shape::Notch,
        Shape::BandPass,
        Shape::TiltShelf,
        Shape::AllPass,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Shape::Bell => "Bell",
            Shape::LowShelf => "Low Shelf",
            Shape::HighShelf => "High Shelf",
            Shape::TiltShelf => "Tilt Shelf",
            Shape::LowCut => "Low Cut",
            Shape::HighCut => "High Cut",
            Shape::Notch => "Notch",
            Shape::BandPass => "Band Pass",
            Shape::AllPass => "All Pass",
        }
    }

    /// whether the gain parameter does anything
    pub fn has_gain(self) -> bool {
        matches!(
            self,
            Shape::Bell | Shape::LowShelf | Shape::HighShelf | Shape::TiltShelf
        )
    }

    /// whether the slope parameter does anything
    pub fn has_slope(self) -> bool {
        matches!(self, Shape::LowCut | Shape::HighCut)
    }

    /// q a new band of this shape starts with
    pub fn default_q(self) -> f32 {
        match self {
            Shape::Bell | Shape::BandPass | Shape::AllPass => 1.0,
            Shape::Notch => 4.0,
            _ => std::f32::consts::FRAC_1_SQRT_2,
        }
    }
}

/// nearest supported slope
pub fn snap_slope(db: u8) -> u8 {
    *SLOPES
        .iter()
        .min_by_key(|&&s| (s as i32 - db as i32).abs())
        .unwrap()
}

/// a digital second-order section, a0 = 1
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Biquad {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Biquad {
    pub const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// squared magnitude at a frequency given by its `Phi` terms
    #[inline]
    pub fn mag2(&self, p: &Phi) -> f64 {
        let num = sq(self.b0 + self.b1 + self.b2) * p.0 + sq(self.b0 - self.b1 + self.b2) * p.1
            - 4.0 * self.b0 * self.b2 * p.2;
        let den = sq(1.0 + self.a1 + self.a2) * p.0 + sq(1.0 - self.a1 + self.a2) * p.1
            - 4.0 * self.a2 * p.2;
        num.max(0.0) / den.max(1e-300)
    }
}

/// the frequency terms of a squared biquad magnitude:
/// |b(e^jw)|² = (b0+b1+b2)²·φ0 + (b0-b1+b2)²·φ1 - 4·b0·b2·φ2
#[derive(Clone, Copy, Debug)]
pub struct Phi(f64, f64, f64);

impl Phi {
    /// `w` in radians per sample
    pub fn new(w: f64) -> Self {
        let s = (w * 0.5).sin().powi(2);
        Self(1.0 - s, s, 4.0 * (1.0 - s) * s)
    }

    pub fn at(freq: f32, sample_rate: f32) -> Self {
        Self::new(2.0 * PI * freq as f64 / sample_rate as f64)
    }
}

fn sq(x: f64) -> f64 {
    x * x
}

/// analog second-order prototype n(s)/d(s), s normalised to the band
/// frequency; coefficients are for s², s, 1
#[derive(Clone, Copy, Debug)]
struct Analog {
    n: [f64; 3],
    d: [f64; 3],
}

impl Analog {
    /// squared magnitude at normalised frequency x
    fn mag2(&self, x: f64) -> f64 {
        let x2 = x * x;
        let num = sq(self.n[2] - self.n[0] * x2) + sq(self.n[1] * x);
        let den = sq(self.d[2] - self.d[0] * x2) + sq(self.d[1] * x);
        num / den.max(1e-300)
    }

    /// digital poles with the same position as the analog ones (impulse
    /// invariant), as (a1, a2)
    fn poles(&self, w0: f64) -> (f64, f64) {
        let [d2, d1, d0] = self.d;
        // pole frequency and damping; kept below nyquist
        let wp = (w0 * (d0 / d2).sqrt()).min(0.95 * PI);
        let zeta = d1 / (2.0 * (d0 * d2).sqrt());
        if zeta < 1.0 {
            let q = (-zeta * wp).exp();
            (-2.0 * q * ((1.0 - zeta * zeta).sqrt() * wp).cos(), q * q)
        } else {
            // two real poles
            let r = (zeta * zeta - 1.0).sqrt();
            let (p1, p2) = (((-zeta + r) * wp).exp(), ((-zeta - r) * wp).exp());
            (-(p1 + p2), p1 * p2)
        }
    }

    /// matched digital section: exact poles, magnitude equal to the analog
    /// response at dc, nyquist and `w0`
    fn matched(&self, w0: f64) -> Biquad {
        let (a1, a2) = self.poles(w0);
        let den0 = sq(1.0 + a1 + a2);
        let den1 = sq(1.0 - a1 + a2);
        let den2 = -4.0 * a2;
        let p = Phi::new(w0);
        let b0_ = den0 * self.mag2(0.0);
        let b1_ = den1 * self.mag2(PI / w0);
        let den_m = den0 * p.0 + den1 * p.1 + den2 * p.2;
        let b2_ = (self.mag2(1.0) * den_m - b0_ * p.0 - b1_ * p.1) / p.2;
        // back from the squared-magnitude terms to coefficients
        let (r0, r1) = (b0_.max(0.0).sqrt(), b1_.max(0.0).sqrt());
        let w = 0.5 * (r0 + r1);
        let b1 = 0.5 * (r0 - r1);
        let b0 = 0.5 * (w + (w * w + b2_).max(0.0).sqrt());
        Biquad {
            b0,
            b1,
            b2: w - b0,
            a1,
            a2,
        }
    }
}

/// matched first-order low or high pass (6 db/oct sections)
fn first_order(w0: f64, highpass: bool) -> Biquad {
    let p = (-w0).exp();
    let x = PI / w0;
    let nyquist = if highpass { x } else { 1.0 } / (1.0 + x * x).sqrt();
    // dc gain (b0 + b1) / (1 - p), nyquist gain (b0 - b1) / (1 + p)
    let dc = if highpass { 0.0 } else { 1.0 - p };
    let ny = (1.0 + p) * nyquist;
    Biquad {
        b0: 0.5 * (dc + ny),
        b1: 0.5 * (dc - ny),
        b2: 0.0,
        a1: -p,
        a2: 0.0,
    }
}

/// a band designed into cascaded sections
#[derive(Clone, Copy, Debug)]
pub struct Design {
    pub sections: [Biquad; MAX_SECTIONS],
    pub len: usize,
}

impl Design {
    fn new() -> Self {
        Self {
            sections: [Biquad::IDENTITY; MAX_SECTIONS],
            len: 0,
        }
    }

    fn push(&mut self, b: Biquad) {
        if self.len < MAX_SECTIONS {
            self.sections[self.len] = b;
            self.len += 1;
        }
    }

    pub fn sections(&self) -> &[Biquad] {
        &self.sections[..self.len]
    }

    pub fn mag2(&self, p: &Phi) -> f64 {
        self.sections().iter().map(|s| s.mag2(p)).product()
    }

    pub fn magnitude_db(&self, p: &Phi) -> f32 {
        (10.0 * self.mag2(p).max(1e-30).log10()) as f32
    }
}

/// designs one band. `slope` is in db/oct and only used by the cuts.
pub fn design(
    shape: Shape,
    slope: u8,
    freq: f32,
    gain_db: f32,
    q: f32,
    sample_rate: f32,
) -> Design {
    let fs = sample_rate as f64;
    let f0 = (freq as f64).clamp(FREQ_MIN as f64, (FREQ_MAX as f64).min(fs * 0.46));
    let w0 = 2.0 * PI * f0 / fs;
    let q = (q as f64).clamp(Q_MIN as f64, Q_MAX as f64);
    let gain = (gain_db as f64).clamp(-GAIN_MAX as f64, GAIN_MAX as f64);
    let a = 10f64.powf(gain / 40.0);
    let sa = a.sqrt();
    let mut d = Design::new();
    let section = |n: [f64; 3], dd: [f64; 3]| Analog { n, d: dd }.matched(w0);
    match shape {
        Shape::Bell => d.push(section([1.0, a / q, 1.0], [1.0, 1.0 / (a * q), 1.0])),
        Shape::LowShelf => d.push(section([a, a * sa / q, a * a], [a, sa / q, 1.0])),
        Shape::HighShelf => d.push(section([a * a, a * sa / q, a], [1.0, sa / q, a])),
        // high shelf of the full gain, shifted down by half of it
        Shape::TiltShelf => d.push(section([a, sa / q, 1.0], [1.0, sa / q, a])),
        Shape::Notch => {
            // zeros exactly on the unit circle; unity gain on the far side
            let (a1, a2) = Analog {
                n: [0.0; 3],
                d: [1.0, 1.0 / q, 1.0],
            }
            .poles(w0);
            let cos = w0.cos();
            let c = if w0 < PI / 2.0 {
                (1.0 + a1 + a2) / (2.0 * (1.0 - cos))
            } else {
                (1.0 - a1 + a2) / (2.0 * (1.0 + cos))
            };
            d.push(Biquad {
                b0: c,
                b1: -2.0 * c * cos,
                b2: c,
                a1,
                a2,
            });
        }
        Shape::BandPass => d.push(section([0.0, 1.0 / q, 0.0], [1.0, 1.0 / q, 1.0])),
        Shape::AllPass => {
            let (a1, a2) = Analog {
                n: [0.0; 3],
                d: [1.0, 1.0 / q, 1.0],
            }
            .poles(w0);
            d.push(Biquad {
                b0: a2,
                b1: a1,
                b2: 1.0,
                a1,
                a2,
            });
        }
        Shape::LowCut | Shape::HighCut => {
            let high = shape == Shape::LowCut;
            let order = (snap_slope(slope) / 6) as usize;
            let pairs = order / 2;
            for k in 0..pairs {
                // butterworth pole pairs; the sharpest one carries the resonance
                let angle = if order.is_multiple_of(2) {
                    PI * (2 * k + 1) as f64 / (2 * order) as f64
                } else {
                    PI * (k + 1) as f64 / order as f64
                };
                let mut qk = 1.0 / (2.0 * angle.cos());
                if k == pairs - 1 {
                    qk *= q / std::f64::consts::FRAC_1_SQRT_2;
                }
                let n = if high {
                    [1.0, 0.0, 0.0]
                } else {
                    [0.0, 0.0, 1.0]
                };
                d.push(section(n, [1.0, 1.0 / qk, 1.0]));
            }
            if order % 2 == 1 {
                d.push(first_order(w0, high));
            }
        }
    }
    d
}

/// coefficients of a trapezoidal svf whose output is
/// `m0·input + m1·bandpass + m2·lowpass`
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SvfCoeffs {
    pub g: f64,
    pub k: f64,
    pub m0: f64,
    pub m1: f64,
    pub m2: f64,
}

impl SvfCoeffs {
    pub const IDENTITY: Self = Self {
        g: 1.0,
        k: 1.0,
        m0: 1.0,
        m1: 0.0,
        m2: 0.0,
    };

    /// the same transfer function as a stable biquad: undo the bilinear
    /// transform, normalise the denominator to s² + k·s + 1 (s scaled by g)
    /// and express the numerator in the svf's outputs
    pub fn from_biquad(b: &Biquad) -> Self {
        let p = 1.0 + b.a1 + b.a2;
        let m = 1.0 - b.a1 + b.a2;
        if p <= 0.0 || m <= 0.0 {
            return Self::IDENTITY;
        }
        let g = (p / m).sqrt();
        let k = 2.0 * (1.0 - b.a2) / (m * g);
        let n2 = (b.b0 - b.b1 + b.b2) * g * g / p;
        let n1 = 2.0 * (b.b0 - b.b2) * g / p;
        let n0 = (b.b0 + b.b1 + b.b2) / p;
        Self {
            g,
            k,
            m0: n2,
            m1: n1 - n2 * k,
            m2: n0 - n2,
        }
    }

    /// plain bandpass with unity gain at `freq`, for detectors and solo
    pub fn bandpass(freq: f32, q: f32, sample_rate: f32) -> Self {
        let f = (freq as f64).clamp(FREQ_MIN as f64, sample_rate as f64 * 0.46);
        let k = 1.0 / (q as f64).clamp(0.1, Q_MAX as f64);
        Self {
            g: (PI * f / sample_rate as f64).tan(),
            k,
            m0: 0.0,
            m1: k,
            m2: 0.0,
        }
    }

    fn lerp_step(from: &Self, to: &Self, n: f64) -> Self {
        Self {
            g: (to.g - from.g) / n,
            k: (to.k - from.k) / n,
            m0: (to.m0 - from.m0) / n,
            m1: (to.m1 - from.m1) / n,
            m2: (to.m2 - from.m2) / n,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Svf {
    c: SvfCoeffs,
    target: SvfCoeffs,
    step: SvfCoeffs,
    ramp: usize,
    a1: f64,
    a2: f64,
    a3: f64,
    ic1: f64,
    ic2: f64,
}

impl Default for Svf {
    fn default() -> Self {
        let mut s = Self {
            c: SvfCoeffs::IDENTITY,
            target: SvfCoeffs::IDENTITY,
            step: SvfCoeffs::IDENTITY,
            ramp: 0,
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            ic1: 0.0,
            ic2: 0.0,
        };
        s.derive();
        s
    }
}

impl Svf {
    fn derive(&mut self) {
        let c = &self.c;
        self.a1 = 1.0 / (1.0 + c.g * (c.g + c.k));
        self.a2 = c.g * self.a1;
        self.a3 = c.g * self.a2;
    }

    pub fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }

    /// jump to new coefficients
    pub fn snap(&mut self, c: SvfCoeffs) {
        self.c = c;
        self.target = c;
        self.ramp = 0;
        self.derive();
    }

    /// glide to new coefficients over the next `samples` samples
    pub fn glide(&mut self, c: SvfCoeffs, samples: usize) {
        if c == self.target && self.ramp == 0 {
            return;
        }
        if samples == 0 {
            return self.snap(c);
        }
        self.target = c;
        self.step = SvfCoeffs::lerp_step(&self.c, &c, samples as f64);
        self.ramp = samples;
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        if self.ramp > 0 {
            self.ramp -= 1;
            if self.ramp == 0 {
                self.c = self.target;
            } else {
                let (c, d) = (&mut self.c, &self.step);
                c.g += d.g;
                c.k += d.k;
                c.m0 += d.m0;
                c.m1 += d.m1;
                c.m2 += d.m2;
            }
            self.derive();
        }
        let v0 = x as f64;
        let v3 = v0 - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        let c = &self.c;
        (c.m0 * v0 + c.m1 * v1 + c.m2 * v2) as f32
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        for s in buf.iter_mut() {
            *s = self.tick(*s);
        }
        // flush denormals once per block
        if self.ic1.abs() < 1e-30 {
            self.ic1 = 0.0;
        }
        if self.ic2.abs() < 1e-30 {
            self.ic2 = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f32 = 48_000.0;

    fn db(d: &Design, f: f32) -> f32 {
        d.magnitude_db(&Phi::at(f, FS))
    }

    /// exact analog magnitude of the same prototypes, for comparison
    fn analog_db(shape: Shape, f0: f32, gain: f32, q: f32, f: f32) -> f32 {
        let a = 10f64.powf(gain as f64 / 40.0);
        let sa = a.sqrt();
        let q = q as f64;
        let p = match shape {
            Shape::Bell => Analog {
                n: [1.0, a / q, 1.0],
                d: [1.0, 1.0 / (a * q), 1.0],
            },
            Shape::LowShelf => Analog {
                n: [a, a * sa / q, a * a],
                d: [a, sa / q, 1.0],
            },
            Shape::HighShelf => Analog {
                n: [a * a, a * sa / q, a],
                d: [1.0, sa / q, a],
            },
            _ => unreachable!(),
        };
        (10.0 * p.mag2(f as f64 / f0 as f64).log10()) as f32
    }

    #[test]
    fn bell_hits_its_gain_without_cramping() {
        for (f0, gain, q) in [
            (1000.0, 6.0, 1.0),
            (100.0, -12.0, 4.0),
            (12_000.0, 9.0, 0.7),
            (18_000.0, -6.0, 2.0),
        ] {
            let d = design(Shape::Bell, 12, f0, gain, q, FS);
            assert!((db(&d, f0) - gain).abs() < 0.01, "{f0} {gain}");
            assert!(db(&d, 10.0).abs() < 0.05);
            // stays close to the analog curve, also where bilinear designs
            // squash it (rbj misses the 18 khz bell by ~3.8 db, this by ~1.5)
            let tolerance = if f0 > 12_000.0 { 1.6 } else { 0.7 };
            for f in [f0 * 0.5, f0 * 0.8, f0 * 1.2, (f0 * 1.5).min(23_000.0)] {
                let err = (db(&d, f) - analog_db(Shape::Bell, f0, gain, q, f)).abs();
                assert!(err < tolerance, "f0 {f0} at {f}: {err}");
            }
        }
    }

    #[test]
    fn shelves_reach_full_gain_and_follow_analog() {
        let lo = design(Shape::LowShelf, 12, 200.0, -4.0, 0.707, FS);
        assert!((db(&lo, 10.0) + 4.0).abs() < 0.05);
        assert!(db(&lo, 20_000.0).abs() < 0.05);
        let hi = design(Shape::HighShelf, 12, 8000.0, 6.0, 0.707, FS);
        assert!(db(&hi, 20.0).abs() < 0.05);
        for f in [2000.0, 8000.0, 15_000.0] {
            let err = (db(&hi, f) - analog_db(Shape::HighShelf, 8000.0, 6.0, 0.707, f)).abs();
            assert!(err < 0.3, "{f}: {err}");
        }
        let tilt = design(Shape::TiltShelf, 12, 1000.0, 6.0, 0.707, FS);
        assert!((db(&tilt, 10.0) + 3.0).abs() < 0.05);
        assert!((db(&tilt, 22_000.0) - 3.0).abs() < 0.1);
    }

    #[test]
    fn cuts_have_their_slope() {
        for slope in SLOPES {
            let d = design(Shape::LowCut, slope, 1000.0, 0.0, 0.707, FS);
            assert_eq!(d.len, (slope as usize / 6).div_ceil(2));
            // butterworth: -3 db at the cutoff, flat above
            assert!(
                (db(&d, 1000.0) + 3.01).abs() < 0.2,
                "{slope}: {}",
                db(&d, 1000.0)
            );
            assert!(db(&d, 16_000.0).abs() < 0.1);
            // one octave further down the slope applies
            let drop = db(&d, 125.0) - db(&d, 250.0);
            assert!((drop + slope as f32).abs() < 1.0, "{slope}: {drop}");
        }
        let hc = design(Shape::HighCut, 24, 5000.0, 0.0, 0.707, FS);
        assert!((db(&hc, 5000.0) + 3.01).abs() < 0.2);
        assert!(db(&hc, 100.0).abs() < 0.01);
        let resonant = design(Shape::LowCut, 12, 1000.0, 0.0, 4.0, FS);
        assert!(db(&resonant, 1000.0) > 11.0);
    }

    #[test]
    fn notch_bandpass_allpass() {
        let n = design(Shape::Notch, 12, 3000.0, 0.0, 4.0, FS);
        assert!(db(&n, 3000.0) < -60.0);
        assert!(db(&n, 1000.0).abs() < 0.3);
        let bp = design(Shape::BandPass, 12, 3000.0, 0.0, 2.0, FS);
        assert!(db(&bp, 3000.0).abs() < 0.01);
        assert!(db(&bp, 300.0) < -20.0);
        let ap = design(Shape::AllPass, 12, 3000.0, 0.0, 1.0, FS);
        for f in [20.0, 3000.0, 20_000.0] {
            assert!(db(&ap, f).abs() < 1e-6);
        }
    }

    #[test]
    fn extremes_stay_finite_and_stable() {
        for shape in Shape::ALL {
            for (f, g, q) in [
                (10.0, 30.0, 0.025),
                (10.0, -30.0, 40.0),
                (22_000.0, 30.0, 40.0),
                (22_000.0, -30.0, 0.025),
            ] {
                let d = design(shape, 96, f, g, q, FS);
                for s in d.sections() {
                    let c = [s.b0, s.b1, s.b2, s.a1, s.a2];
                    assert!(c.iter().all(|v| v.is_finite()), "{shape:?} {f} {g} {q}");
                    assert!(s.a2.abs() < 1.0 && s.a1.abs() < 1.0 + s.a2);
                }
            }
        }
    }

    /// runs a sine through svfs built from a design, returns the gain in db
    fn measured_db(d: &Design, f: f32) -> f32 {
        let mut svfs: Vec<Svf> = d
            .sections()
            .iter()
            .map(|b| {
                let mut s = Svf::default();
                s.snap(SvfCoeffs::from_biquad(b));
                s
            })
            .collect();
        let mut peak = 0.0f32;
        for n in 0..FS as usize {
            let mut y = (2.0 * std::f32::consts::PI * f * n as f32 / FS).sin();
            for s in &mut svfs {
                y = s.tick(y);
            }
            if n > FS as usize / 2 {
                peak = peak.max(y.abs());
            }
        }
        20.0 * peak.log10()
    }

    #[test]
    fn svf_matches_designed_response() {
        for (shape, f0, gain, q) in [
            (Shape::Bell, 1000.0, 12.0, 2.0),
            (Shape::HighShelf, 6000.0, -6.0, 0.7),
            (Shape::LowCut, 200.0, 0.0, 0.7),
        ] {
            let d = design(shape, 18, f0, gain, q, FS);
            for f in [100.0, 1000.0, 5000.0] {
                let err = (measured_db(&d, f) - db(&d, f)).abs();
                assert!(err < 0.05, "{shape:?} at {f}: {err}");
            }
        }
    }

    #[test]
    fn gliding_is_smooth() {
        let a = SvfCoeffs::from_biquad(&design(Shape::Bell, 12, 200.0, -12.0, 4.0, FS).sections[0]);
        let b = SvfCoeffs::from_biquad(&design(Shape::Bell, 12, 8000.0, 12.0, 0.5, FS).sections[0]);
        let mut s = Svf::default();
        s.snap(a);
        let mut last = 0.0f32;
        let mut max_step = 0.0f32;
        for n in 0..4800 {
            if n % 32 == 0 {
                s.glide(if (n / 32) % 2 == 0 { b } else { a }, 32);
            }
            let y = s.tick((2.0 * std::f32::consts::PI * 100.0 * n as f32 / FS).sin() * 0.5);
            assert!(y.is_finite());
            max_step = max_step.max((y - last).abs());
            last = y;
        }
        assert!(max_step < 0.2, "{max_step}");
    }
}
