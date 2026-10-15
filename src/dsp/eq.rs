//! parametric eq with up to 24 bands, dynamic bands, band solo and
//! spectrum analyzer taps. zero latency.
//!
//! the gui writes band parameters into atomics; the audio thread works in
//! chunks of `CHUNK` samples, smooths the parameters, runs the dynamics
//! detectors and redesigns the sections that changed. coefficients glide
//! across each chunk, and changes of shape or slope crossfade the band out
//! and back in, so nothing the gui does can click.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicUsize, Ordering};

use super::filter::{self, Design, MAX_SECTIONS, Shape, Svf, SvfCoeffs};
use super::{AtomicF32, db_to_lin, lin_to_db};

pub const MAX_BANDS: usize = 24;
/// control-rate period in samples (0.67 ms at 48 khz)
const CHUNK: usize = 32;
/// band fade time for on/off and shape changes
const FADE_MS: f32 = 10.0;
/// parameter smoothing time constant
const SMOOTH_MS: f32 = 15.0;
/// soft knee of the dynamics
const KNEE_DB: f32 = 6.0;
const NO_SOLO: u8 = u8::MAX;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dynamics {
    pub on: bool,
    /// detector level above which the gain starts to move
    pub threshold: f32,
    /// maximum gain change; negative compresses the band, positive expands
    pub range: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
}

impl Default for Dynamics {
    fn default() -> Self {
        Self {
            on: false,
            threshold: -30.0,
            range: -6.0,
            attack_ms: 5.0,
            release_ms: 100.0,
        }
    }
}

/// plain-data snapshot of one band
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Band {
    pub on: bool,
    pub shape: Shape,
    /// db/oct, cuts only
    pub slope: u8,
    pub freq: f32,
    pub gain: f32,
    pub q: f32,
    pub dynamics: Dynamics,
}

impl Band {
    pub fn new(shape: Shape, freq: f32, gain: f32) -> Self {
        Self {
            on: true,
            shape,
            slope: 12,
            freq,
            gain,
            q: shape.default_q(),
            dynamics: Dynamics::default(),
        }
    }

    pub fn clamped(mut self) -> Self {
        self.freq = self.freq.clamp(filter::FREQ_MIN, filter::FREQ_MAX);
        self.gain = self.gain.clamp(-filter::GAIN_MAX, filter::GAIN_MAX);
        self.q = self.q.clamp(filter::Q_MIN, filter::Q_MAX);
        self.slope = filter::snap_slope(self.slope);
        let d = &mut self.dynamics;
        d.threshold = d.threshold.clamp(-80.0, 0.0);
        d.range = d.range.clamp(-filter::GAIN_MAX, filter::GAIN_MAX);
        d.attack_ms = d.attack_ms.clamp(0.5, 500.0);
        d.release_ms = d.release_ms.clamp(5.0, 2000.0);
        self
    }

    pub fn is_dynamic(&self) -> bool {
        self.dynamics.on && self.shape.has_gain()
    }

    pub fn design(&self, gain: f32, sample_rate: f32) -> Design {
        filter::design(self.shape, self.slope, self.freq, gain, self.q, sample_rate)
    }
}

/// what a fresh install starts with: the old default high-pass
pub fn default_bands() -> Vec<Band> {
    vec![Band {
        q: std::f32::consts::FRAC_1_SQRT_2,
        ..Band::new(Shape::LowCut, 80.0, 0.0)
    }]
}

#[derive(Debug, Default)]
pub struct BandParams {
    /// the slot holds a band
    pub used: AtomicBool,
    pub on: AtomicBool,
    pub shape: AtomicU8,
    pub slope: AtomicU8,
    pub freq: AtomicF32,
    pub gain: AtomicF32,
    pub q: AtomicF32,
    pub dyn_on: AtomicBool,
    pub threshold: AtomicF32,
    pub range: AtomicF32,
    pub attack: AtomicF32,
    pub release: AtomicF32,
    /// written by the audio thread: detector level (dbfs) and applied gain
    pub level: AtomicF32,
    pub live_gain: AtomicF32,
}

/// a lock-free ring the audio thread writes recent samples into, for the
/// analyzer. the reader may see a sample torn by a concurrent write, which
/// only matters for a single analyzer frame.
pub struct Tap {
    buf: Box<[AtomicU32]>,
    pos: AtomicUsize,
}

impl Tap {
    pub const LEN: usize = 1 << 15;

    fn new() -> Self {
        Self {
            buf: (0..Self::LEN).map(|_| AtomicU32::new(0)).collect(),
            pos: AtomicUsize::new(0),
        }
    }

    fn write(&self, x: &[f32]) {
        let p = self.pos.load(Ordering::Relaxed);
        for (i, s) in x.iter().enumerate() {
            self.buf[(p + i) & (Self::LEN - 1)].store(s.to_bits(), Ordering::Relaxed);
        }
        self.pos.store(p.wrapping_add(x.len()), Ordering::Release);
    }

    /// copies the newest `out.len()` samples; returns the write position
    pub fn read(&self, out: &mut [f32]) -> usize {
        let p = self.pos.load(Ordering::Acquire);
        let start = p.wrapping_sub(out.len());
        for (i, o) in out.iter_mut().enumerate() {
            let v = self.buf[start.wrapping_add(i) & (Self::LEN - 1)].load(Ordering::Relaxed);
            *o = f32::from_bits(v);
        }
        p
    }
}

pub struct EqParams {
    pub bands: [BandParams; MAX_BANDS],
    pub output_gain: AtomicF32,
    /// index of the soloed band, `NO_SOLO` if none
    solo: AtomicU8,
    /// the gui is showing the analyzer; taps are only fed while set
    pub analyzer: AtomicBool,
    pub pre: Tap,
    pub post: Tap,
}

impl EqParams {
    pub fn new(bands: &[Band], output_gain: f32) -> Self {
        let p = Self {
            bands: Default::default(),
            output_gain: AtomicF32::new(output_gain),
            solo: AtomicU8::new(NO_SOLO),
            analyzer: AtomicBool::new(false),
            pre: Tap::new(),
            post: Tap::new(),
        };
        for (i, b) in bands.iter().take(MAX_BANDS).enumerate() {
            p.set_band(i, Some(*b));
        }
        p
    }

    pub fn band(&self, i: usize) -> Option<Band> {
        let p = &self.bands[i];
        if !p.used.load(Ordering::Relaxed) {
            return None;
        }
        Some(Band {
            on: p.on.load(Ordering::Relaxed),
            shape: Shape::ALL
                .get(p.shape.load(Ordering::Relaxed) as usize)
                .copied()
                .unwrap_or_default(),
            slope: p.slope.load(Ordering::Relaxed),
            freq: p.freq.get(),
            gain: p.gain.get(),
            q: p.q.get(),
            dynamics: Dynamics {
                on: p.dyn_on.load(Ordering::Relaxed),
                threshold: p.threshold.get(),
                range: p.range.get(),
                attack_ms: p.attack.get(),
                release_ms: p.release.get(),
            },
        })
    }

    /// `None` removes the band
    pub fn set_band(&self, i: usize, band: Option<Band>) {
        let p = &self.bands[i];
        let Some(b) = band.map(Band::clamped) else {
            p.used.store(false, Ordering::Relaxed);
            if self.solo() == Some(i) {
                self.set_solo(None);
            }
            return;
        };
        let shape = Shape::ALL.iter().position(|&s| s == b.shape).unwrap_or(0);
        p.on.store(b.on, Ordering::Relaxed);
        p.shape.store(shape as u8, Ordering::Relaxed);
        p.slope.store(b.slope, Ordering::Relaxed);
        p.freq.set(b.freq);
        p.gain.set(b.gain);
        p.q.set(b.q);
        p.dyn_on.store(b.dynamics.on, Ordering::Relaxed);
        p.threshold.set(b.dynamics.threshold);
        p.range.set(b.dynamics.range);
        p.attack.set(b.dynamics.attack_ms);
        p.release.set(b.dynamics.release_ms);
        if !p.used.swap(true, Ordering::Relaxed) {
            p.live_gain.set(b.gain);
            p.level.set(-120.0);
        }
    }

    /// puts a band into the first free slot
    pub fn add_band(&self, band: Band) -> Option<usize> {
        let i = (0..MAX_BANDS).find(|&i| self.band(i).is_none())?;
        self.set_band(i, Some(band));
        Some(i)
    }

    /// used bands in slot order
    pub fn snapshot(&self) -> Vec<Band> {
        (0..MAX_BANDS).filter_map(|i| self.band(i)).collect()
    }

    pub fn solo(&self) -> Option<usize> {
        let s = self.solo.load(Ordering::Relaxed);
        (s != NO_SOLO).then_some(s as usize)
    }

    pub fn set_solo(&self, band: Option<usize>) {
        self.solo
            .store(band.map_or(NO_SOLO, |b| b as u8), Ordering::Relaxed);
    }

    /// gain currently applied by a band, including its dynamics
    pub fn live_gain(&self, i: usize) -> f32 {
        self.bands[i].live_gain.get()
    }

    /// dynamics detector level of a band in dbfs
    pub fn level(&self, i: usize) -> f32 {
        self.bands[i].level.get()
    }
}

/// what a band is built from; changing it needs a crossfade
#[derive(Clone, Copy, Debug, PartialEq)]
struct Structure {
    shape: Shape,
    slope: u8,
}

struct BandDsp {
    live: Option<Structure>,
    fade: f32,
    sections: [Svf; MAX_SECTIONS],
    len: usize,
    /// smoothed parameters; freq is smoothed in log2
    log_freq: f32,
    gain: f32,
    q: f32,
    /// parameters the sections were last designed for
    designed: Option<(f32, f32, f32)>,
    detector: Svf,
    detector_for: Option<(f32, f32)>,
    /// detector envelope, linear peak
    env: f32,
}

impl Default for BandDsp {
    fn default() -> Self {
        Self {
            live: None,
            fade: 0.0,
            sections: Default::default(),
            len: 0,
            log_freq: 0.0,
            gain: 0.0,
            q: 1.0,
            designed: None,
            detector: Svf::default(),
            detector_for: None,
            env: 0.0,
        }
    }
}

impl BandDsp {
    fn reset(&mut self) {
        for s in &mut self.sections {
            s.reset();
        }
        self.detector.reset();
        self.env = 0.0;
    }
}

pub struct Eq {
    params: Arc<EqParams>,
    fs: f32,
    bands: Box<[BandDsp; MAX_BANDS]>,
    fade_step: f32,
    smooth: f32,
    out_gain: f32,
    solo_fade: f32,
    solo_band: Option<usize>,
}

impl Eq {
    pub fn new(params: Arc<EqParams>, sample_rate: f32) -> Self {
        Self {
            params,
            fs: sample_rate,
            bands: Box::new(std::array::from_fn(|_| BandDsp::default())),
            fade_step: 1.0 / (FADE_MS * 0.001 * sample_rate),
            smooth: 1.0 - (-(CHUNK as f32) / (SMOOTH_MS * 0.001 * sample_rate)).exp(),
            out_gain: 1.0,
            solo_fade: 0.0,
            solo_band: None,
        }
    }

    pub fn reset(&mut self) {
        for b in self.bands.iter_mut() {
            b.reset();
        }
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        for chunk in buf.chunks_mut(CHUNK) {
            self.process_chunk(chunk);
        }
    }

    fn process_chunk(&mut self, buf: &mut [f32]) {
        let n = buf.len();
        let mut dry = [0.0f32; CHUNK];
        dry[..n].copy_from_slice(buf);
        let dry = &dry[..n];
        let mut solo = [0.0f32; CHUNK];

        let soloed = self.params.solo();
        if soloed.is_some() {
            self.solo_band = soloed;
        }
        for i in 0..MAX_BANDS {
            let want_solo = self.solo_band == Some(i);
            self.process_band(i, dry, buf, want_solo.then_some(&mut solo[..n]));
        }

        // solo: crossfade to the soloed band's region of the dry signal
        let solo_target = if soloed.is_some() { 1.0 } else { 0.0 };
        if self.solo_fade > 0.0 || solo_target > 0.0 {
            for (o, s) in buf.iter_mut().zip(&solo[..n]) {
                self.solo_fade = step_toward(self.solo_fade, solo_target, self.fade_step);
                *o += (*s - *o) * self.solo_fade;
            }
            if self.solo_fade == 0.0 {
                self.solo_band = None;
            }
        }

        let target = db_to_lin(self.params.output_gain.get().clamp(-30.0, 30.0));
        if target != 1.0 || self.out_gain != 1.0 {
            let step = (target - self.out_gain) / n as f32;
            for o in buf.iter_mut() {
                self.out_gain += step;
                *o *= self.out_gain;
            }
            self.out_gain = target;
        }

        if self.params.analyzer.load(Ordering::Relaxed) {
            self.params.pre.write(dry);
            self.params.post.write(buf);
        }
    }

    fn process_band(&mut self, i: usize, dry: &[f32], buf: &mut [f32], solo: Option<&mut [f32]>) {
        let n = buf.len();
        let p = &self.params.bands[i];
        let band = self.params.band(i);
        let b = &mut self.bands[i];
        let want = band.map(|b| Structure {
            shape: b.shape,
            slope: b.slope,
        });

        // structural changes wait until the band has faded out
        let mut snap = false;
        if b.live != want && b.fade == 0.0 {
            b.live = want;
            b.len = 0;
            b.designed = None;
            b.reset();
            snap = true;
        }
        let Some(band) = band.filter(|_| b.live.is_some()) else {
            // removed: keep fading out with the old sections, then idle
            if b.fade > 0.0 {
                run_sections(b, buf, 0.0, self.fade_step);
            }
            return;
        };
        let fade_target = if b.live == want && band.on { 1.0 } else { 0.0 };
        if b.fade == 0.0 && fade_target == 0.0 && solo.is_none() {
            b.designed = None;
            b.env = 0.0;
            p.live_gain.set(band.gain);
            return;
        }

        // smooth the continuous parameters, snapping when a band (re)starts
        let lf = band.freq.log2();
        if b.designed.is_none() {
            (b.log_freq, b.gain, b.q) = (lf, band.gain, band.q);
            snap = true;
        } else {
            b.log_freq = smooth(b.log_freq, lf, self.smooth, 1e-4);
            b.gain = smooth(b.gain, band.gain, self.smooth, 1e-3);
            b.q = smooth(b.q, band.q, self.smooth, 1e-4);
        }
        let freq = b.log_freq.exp2();

        // detector: the band's region of the dry input
        let dynamic = band.is_dynamic();
        let mut delta = 0.0;
        if dynamic || solo.is_some() {
            let q = if band.shape == Shape::Bell || band.shape == Shape::Notch {
                b.q.clamp(0.3, 8.0)
            } else {
                0.7
            };
            if b.detector_for != Some((freq, q)) {
                b.detector.glide(SvfCoeffs::bandpass(freq, q, self.fs), n);
                b.detector_for = Some((freq, q));
            }
            let mut peak = 0.0f32;
            let mut out = [0.0f32; CHUNK];
            for (o, &x) in out.iter_mut().zip(dry) {
                *o = b.detector.tick(x);
                peak = peak.max(o.abs());
            }
            if let Some(solo) = solo {
                solo.copy_from_slice(&out[..n]);
            }
            let d = band.dynamics;
            let ms = if peak > b.env {
                d.attack_ms
            } else {
                d.release_ms
            };
            let coef = 1.0 - (-(n as f32) / (ms * 0.001 * self.fs)).exp();
            b.env += (peak - b.env) * coef;
            if b.env < 1e-9 {
                b.env = 0.0;
            }
            let level = lin_to_db(b.env);
            p.level.set(level);
            if dynamic {
                let over = soft_knee(level - d.threshold);
                delta = over.min(d.range.abs()) * d.range.signum();
            }
        }
        if !dynamic {
            b.env = 0.0;
        }
        let gain = b.gain + delta;
        p.live_gain.set(gain);

        let key = (freq, gain, b.q);
        if b.designed != Some(key) {
            let s = b.live.unwrap();
            let d = filter::design(s.shape, s.slope, freq, gain, b.q, self.fs);
            for (svf, section) in b.sections.iter_mut().zip(d.sections()) {
                let c = SvfCoeffs::from_biquad(section);
                if snap {
                    svf.snap(c);
                } else {
                    svf.glide(c, n);
                }
            }
            b.len = d.len;
            b.designed = Some(key);
        }
        run_sections(b, buf, fade_target, self.fade_step);
    }
}

/// runs a band's sections in series over `buf`, crossfading with the
/// band's input while it fades
fn run_sections(b: &mut BandDsp, buf: &mut [f32], target: f32, step: f32) {
    let n = buf.len();
    let mut x = [0.0f32; CHUNK];
    x[..n].copy_from_slice(buf);
    for s in &mut b.sections[..b.len] {
        s.process(buf);
    }
    if b.fade == 1.0 && target == 1.0 {
        return;
    }
    for (o, &x) in buf.iter_mut().zip(&x[..n]) {
        b.fade = step_toward(b.fade, target, step);
        *o = x + (*o - x) * b.fade;
    }
}

fn step_toward(v: f32, target: f32, step: f32) -> f32 {
    if v < target {
        (v + step).min(target)
    } else {
        (v - step).max(target)
    }
}

fn smooth(v: f32, target: f32, coef: f32, eps: f32) -> f32 {
    let next = v + (target - v) * coef;
    if (target - next).abs() < eps {
        target
    } else {
        next
    }
}

/// level above threshold with a quadratic knee
fn soft_knee(over: f32) -> f32 {
    let h = KNEE_DB * 0.5;
    if over <= -h {
        0.0
    } else if over >= h {
        over
    } else {
        (over + h) * (over + h) / (2.0 * KNEE_DB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::filter::Phi;

    const FS: f32 = 48_000.0;

    fn sine(f: f32, amp: f32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| amp * (2.0 * std::f32::consts::PI * f * i as f32 / FS).sin())
            .collect()
    }

    fn peak_db(x: &[f32]) -> f32 {
        lin_to_db(super::super::peak(x))
    }

    fn run(eq: &mut Eq, x: &mut [f32]) {
        // odd block sizes, like pipewire quanta
        for c in x.chunks_mut(100) {
            eq.process(c);
        }
    }

    #[test]
    fn bell_applies_its_gain() {
        let params = Arc::new(EqParams::new(&[Band::new(Shape::Bell, 1000.0, 6.0)], 0.0));
        let mut eq = Eq::new(params, FS);
        let mut x = sine(1000.0, 0.25, 48_000);
        run(&mut eq, &mut x);
        assert!((peak_db(&x[24_000..]) - (lin_to_db(0.25) + 6.0)).abs() < 0.05);
    }

    #[test]
    fn output_gain_and_removed_bands() {
        let params = Arc::new(EqParams::new(&[Band::new(Shape::Bell, 1000.0, 6.0)], -6.0));
        let mut eq = Eq::new(params.clone(), FS);
        params.set_band(0, None);
        let mut x = sine(1000.0, 0.5, 48_000);
        run(&mut eq, &mut x);
        assert!((peak_db(&x[24_000..]) - (lin_to_db(0.5) - 6.0)).abs() < 0.05);
        assert!(params.snapshot().is_empty());
    }

    #[test]
    fn shape_changes_do_not_click() {
        let params = Arc::new(EqParams::new(&[Band::new(Shape::Bell, 200.0, 12.0)], 0.0));
        let mut eq = Eq::new(params.clone(), FS);
        let mut x = sine(150.0, 0.5, 48_000);
        let mut out = Vec::new();
        for (k, c) in x.chunks_mut(256).enumerate() {
            match k {
                20 => params.set_band(0, Some(Band::new(Shape::HighCut, 300.0, 0.0))),
                60 => params.set_band(0, Some(Band::new(Shape::Bell, 8000.0, -12.0))),
                100 => params.bands[0].on.store(false, Ordering::Relaxed),
                _ => {}
            }
            eq.process(c);
            out.extend_from_slice(c);
        }
        let max_step = out
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        // a 150 hz sine at +12 db moves at most ~0.05 per sample
        assert!(max_step < 0.06, "{max_step}");
    }

    #[test]
    fn dynamic_band_compresses_only_loud_signals() {
        let mut band = Band::new(Shape::Bell, 4000.0, 0.0);
        band.q = 2.0;
        band.dynamics = Dynamics {
            on: true,
            threshold: -30.0,
            range: -9.0,
            attack_ms: 1.0,
            release_ms: 50.0,
        };
        let params = Arc::new(EqParams::new(&[band], 0.0));
        let mut eq = Eq::new(params.clone(), FS);
        // quiet: below threshold, untouched
        let mut x = sine(4000.0, 0.01, 24_000);
        run(&mut eq, &mut x);
        assert!((peak_db(&x[12_000..]) - lin_to_db(0.01)).abs() < 0.1);
        // loud: 24 db over, the full range applies
        let mut x = sine(4000.0, 0.5, 24_000);
        run(&mut eq, &mut x);
        assert!((peak_db(&x[12_000..]) - (lin_to_db(0.5) - 9.0)).abs() < 0.3);
        assert!((params.live_gain(0) + 9.0).abs() < 0.1);
        // other frequencies are left alone
        let mut x = sine(200.0, 0.5, 24_000);
        run(&mut eq, &mut x);
        assert!((peak_db(&x[12_000..]) - lin_to_db(0.5)).abs() < 0.1);
    }

    #[test]
    fn solo_isolates_the_band() {
        let params = Arc::new(EqParams::new(&[Band::new(Shape::Bell, 1000.0, 0.0)], 0.0));
        let mut eq = Eq::new(params.clone(), FS);
        params.set_solo(Some(0));
        let mut lo = sine(100.0, 0.5, 24_000);
        run(&mut eq, &mut lo);
        let mut mid = sine(1000.0, 0.5, 24_000);
        run(&mut eq, &mut mid);
        assert!(peak_db(&lo[12_000..]) < lin_to_db(0.5) - 12.0);
        assert!((peak_db(&mid[12_000..]) - lin_to_db(0.5)).abs() < 0.2);
    }

    #[test]
    fn processing_matches_drawn_curve() {
        let mut lc = Band::new(Shape::LowCut, 120.0, 0.0);
        lc.slope = 36;
        let bands = [
            lc,
            Band::new(Shape::HighShelf, 6000.0, 4.0),
            Band::new(Shape::Notch, 2000.0, 0.0),
        ];
        let params = Arc::new(EqParams::new(&bands, 0.0));
        let mut eq = Eq::new(params.clone(), FS);
        for f in [90.0, 500.0, 10_000.0] {
            let drawn: f32 = params
                .snapshot()
                .iter()
                .map(|b| b.design(b.gain, FS).magnitude_db(&Phi::at(f, FS)))
                .sum();
            eq.reset();
            let mut x = sine(f, 0.1, 48_000);
            run(&mut eq, &mut x);
            let heard = peak_db(&x[24_000..]) - lin_to_db(0.1);
            assert!((heard - drawn).abs() < 0.1, "{f}: {heard} vs {drawn}");
        }
    }

    #[test]
    fn taps_return_recent_samples() {
        let params = Arc::new(EqParams::new(&[], 0.0));
        params.analyzer.store(true, Ordering::Relaxed);
        let mut eq = Eq::new(params.clone(), FS);
        let mut x: Vec<f32> = (0..Tap::LEN + 1000).map(|i| i as f32).collect();
        run(&mut eq, &mut x);
        let mut out = [0.0; 4];
        params.post.read(&mut out);
        let last = (Tap::LEN + 999) as f32;
        assert_eq!(out, [last - 3.0, last - 2.0, last - 1.0, last]);
    }
}
