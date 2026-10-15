//! eq panel in the style of fabfilter pro-q: an interactive response graph
//! over a real-time spectrum analyzer, a strip of knobs for the selected
//! band, and global controls.
//!
//! graph: double-click to add a band (near the edges: cuts), drag a node
//! to move it (shift: fine; vertical drag on cuts, notches and band
//! passes changes q), scroll over a node for q, double-click a node to
//! bypass it, right-click for its menu, delete/backspace removes the
//! selected band. ctrl/cmd+z undoes, shift+ctrl/cmd+z redoes.

use std::sync::Arc;
use std::time::Instant;

use egui::{Color32, Mesh, Pos2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

use super::knob::Knob;
use super::{AMBER, GREEN, GREY};
use crate::audio::SAMPLE_RATE;
use crate::config::{AnalyzerMode, EqConfig};
use crate::dsp::eq::{Band, EqParams, MAX_BANDS, default_bands};
use crate::dsp::filter::{self, Phi, SLOPES, Shape};

const F_MIN: f32 = 10.0;
const F_MAX: f32 = 22_000.0;
pub const RANGES: [f32; 4] = [3.0, 6.0, 12.0, 30.0];
const FFT_LEN: usize = 8192;
/// analyzer scale: top of the graph and span in dbfs
const ANA_TOP: f32 = 0.0;
const ANA_SPAN: f32 = 90.0;
/// analyzer tilt around 1 khz, so pink-ish material looks flat (as pro-q)
const TILT_DB_PER_OCT: f32 = 4.5;
const NODE_RADIUS: f32 = 8.0;
const UNDO_DEPTH: usize = 100;

type Slots = [Option<Band>; MAX_BANDS];

pub fn band_color(i: usize) -> Color32 {
    // spread hues so neighbouring slots differ clearly
    let hue = (i as f32 * 0.381_966) % 1.0;
    egui::ecolor::Hsva::new(hue, 0.62, 0.95, 1.0).into()
}

fn x_of(rect: Rect, f: f32) -> f32 {
    rect.left() + (f / F_MIN).ln() / (F_MAX / F_MIN).ln() * rect.width()
}

fn f_of(rect: Rect, x: f32) -> f32 {
    let t = ((x - rect.left()) / rect.width()).clamp(0.0, 1.0);
    F_MIN * (F_MAX / F_MIN).powf(t)
}

fn format_freq(f: f32) -> String {
    if f >= 10_000.0 {
        format!("{:.1} kHz", f / 1000.0)
    } else if f >= 1000.0 {
        format!("{:.2} kHz", f / 1000.0)
    } else if f >= 100.0 {
        format!("{f:.0} Hz")
    } else {
        format!("{f:.1} Hz")
    }
}

fn format_db(db: f32) -> String {
    format!("{db:+.1} dB")
}

/// nearest note and its offset in cents, e.g. "A4 +3"
fn note(f: f32) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let midi = 69.0 + 12.0 * (f / 440.0).log2();
    let n = midi.round();
    let cents = ((midi - n) * 100.0).round() as i32;
    let n = n as i32;
    format!(
        "{}{} {cents:+}",
        NAMES[n.rem_euclid(12) as usize],
        n.div_euclid(12) - 1
    )
}

/// smoothed analyzer curve, one value per graph column
#[derive(Default)]
struct Spectrum {
    db: Vec<f32>,
    pos: usize,
}

struct Analyzer {
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    input: Vec<f32>,
    bins: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    power: Vec<f32>,
    /// turns bin power into dbfs of a full-scale sine
    norm: f32,
    pre: Spectrum,
    post: Spectrum,
}

impl Analyzer {
    fn new() -> Self {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(FFT_LEN);
        // blackman-harris: low leakage, so cuts show their real depth
        let window: Vec<f32> = (0..FFT_LEN)
            .map(|i| {
                let x = std::f32::consts::TAU * i as f32 / FFT_LEN as f32;
                0.35875 - 0.48829 * x.cos() + 0.14128 * (2.0 * x).cos() - 0.01168 * (3.0 * x).cos()
            })
            .collect();
        let sum: f32 = window.iter().sum();
        Self {
            input: fft.make_input_vec(),
            bins: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            power: vec![0.0; FFT_LEN / 2 + 1],
            norm: (2.0 / sum).powi(2),
            fft,
            window,
            pre: Spectrum::default(),
            post: Spectrum::default(),
        }
    }

    /// refreshes one curve from a tap; `freqs` are the column frequencies
    fn update(&mut self, post: bool, params: &EqParams, freqs: &[f32], dt: f32) {
        let tap = if post { &params.post } else { &params.pre };
        let pos = tap.read(&mut self.input);
        let fresh = {
            let spec = if post { &mut self.post } else { &mut self.pre };
            let fresh = pos != spec.pos;
            spec.pos = pos;
            if spec.db.len() != freqs.len() {
                spec.db = vec![-200.0; freqs.len()];
            }
            fresh
        };
        if fresh {
            for (x, w) in self.input.iter_mut().zip(&self.window) {
                *x *= w;
            }
            if self
                .fft
                .process_with_scratch(&mut self.input, &mut self.bins, &mut self.scratch)
                .is_err()
            {
                return;
            }
            for (p, b) in self.power.iter_mut().zip(&self.bins) {
                *p = b.norm_sqr() * self.norm;
            }
        }
        let spec = if post { &mut self.post } else { &mut self.pre };
        let df = SAMPLE_RATE as f32 / FFT_LEN as f32;
        let fall = 1.0 - (-dt / 0.25).exp();
        let rise = 1.0 - (-dt / 0.03).exp();
        for (c, &f) in freqs.iter().enumerate() {
            let target = if fresh {
                // column edges halfway (in log) to the neighbours
                let lo = if c > 0 { (freqs[c - 1] * f).sqrt() } else { f };
                let hi = if c + 1 < freqs.len() {
                    (freqs[c + 1] * f).sqrt()
                } else {
                    f
                };
                let (k0, k1) = ((lo / df).floor() as usize, (hi / df).ceil() as usize);
                let p = if k1 > k0 + 1 {
                    self.power[k0.max(1)..k1.min(self.power.len())]
                        .iter()
                        .fold(0.0f32, |m, &v| m.max(v))
                } else {
                    // narrower than a bin: interpolate
                    let k = (f / df).min(self.power.len() as f32 - 2.0);
                    let (i, t) = (k as usize, k.fract());
                    self.power[i] * (1.0 - t) + self.power[i + 1] * t
                };
                10.0 * p.max(1e-20).log10() + TILT_DB_PER_OCT * (f / 1000.0).log2()
            } else {
                // stream stopped (eq switched off or bypassed): let it fall
                -200.0
            };
            let v = &mut spec.db[c];
            *v += (target - *v) * if target > *v { rise } else { fall };
        }
    }
}

struct Drag {
    band: usize,
    start: Band,
    origin: Pos2,
    fine: bool,
}

#[derive(Clone, Copy)]
enum MenuTarget {
    Band(usize),
    Empty { freq: f32, gain: f32 },
}

pub struct EqView {
    pub analyzer_mode: AnalyzerMode,
    /// +- db of the graph
    pub range: f32,
    selected: Option<usize>,
    drag: Option<Drag>,
    menu: Option<MenuTarget>,
    analyzer: Analyzer,
    last_frame: Instant,
    undo: Vec<Slots>,
    redo: Vec<Slots>,
    /// state before the gesture in progress
    gesture: Option<Slots>,
    /// height of everything below the graph, measured last frame
    below_graph: f32,
    /// where the graph was drawn last frame
    graph_rect: Rect,
}

impl EqView {
    pub fn new(config: &EqConfig) -> Self {
        Self {
            analyzer_mode: config.analyzer,
            range: RANGES
                .into_iter()
                .find(|&r| r >= config.display_range)
                .unwrap_or(12.0),
            selected: None,
            drag: None,
            menu: None,
            analyzer: Analyzer::new(),
            last_frame: Instant::now(),
            undo: Vec::new(),
            redo: Vec::new(),
            gesture: None,
            below_graph: 140.0,
            graph_rect: Rect::NOTHING,
        }
    }

    fn y_of(&self, rect: Rect, db: f32) -> f32 {
        rect.center().y - db / self.range * rect.height() * 0.5
    }

    fn db_of(&self, rect: Rect, y: f32) -> f32 {
        (rect.center().y - y) / (rect.height() * 0.5) * self.range
    }

    /// where a band's node sits
    fn node_pos(&self, rect: Rect, b: &Band) -> Pos2 {
        let db = if b.shape.has_gain() { b.gain } else { 0.0 };
        pos2(
            x_of(rect, b.freq),
            self.y_of(rect, db.clamp(-self.range, self.range)),
        )
    }

    /// the analyzer stops being fed when the eq panel isn't visible
    pub fn hidden(&mut self, params: &EqParams) {
        params
            .analyzer
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }

    /// returns true if anything that belongs in the config changed
    pub fn show(&mut self, ui: &mut Ui, params: &EqParams) -> bool {
        let before: Slots = std::array::from_fn(|i| params.band(i));
        let view_before = (self.analyzer_mode, self.range, params.output_gain.get());
        params.analyzer.store(
            self.analyzer_mode != AnalyzerMode::Off,
            std::sync::atomic::Ordering::Relaxed,
        );
        if self.selected.is_some_and(|i| before[i].is_none()) {
            self.selected = None;
        }

        let height = (ui.available_height() - self.below_graph).max(200.0);
        let (rect, resp) =
            ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click_and_drag());
        self.graph_rect = rect;
        self.graph(ui, rect, &resp, params, &before);
        let top = ui.cursor().top();
        ui.add_space(6.0);
        self.band_strip(ui, params);
        ui.add_space(4.0);
        self.bottom_bar(ui, params);
        // the strip wraps on narrow windows; the graph gives way next frame
        let below = ui.cursor().top() - top + 4.0;
        if (below - self.below_graph).abs() > 0.5 {
            self.below_graph = below;
            ui.ctx().request_repaint();
        }
        self.keys(ui, params);

        // group everything until the pointer is released into one undo step
        let after: Slots = std::array::from_fn(|i| params.band(i));
        if after != before && self.gesture.is_none() {
            self.gesture = Some(before);
        }
        if !ui.input(|i| i.pointer.any_down())
            && let Some(g) = self.gesture.take()
            && g != after
        {
            self.undo.push(g);
            if self.undo.len() > UNDO_DEPTH {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        after != before || view_before != (self.analyzer_mode, self.range, params.output_gain.get())
    }

    fn graph(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        resp: &egui::Response,
        params: &EqParams,
        bands: &Slots,
    ) {
        let fs = SAMPLE_RATE as f32;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, Color32::from_rgb(0x16, 0x19, 0x1e));

        // columns: one per two pixels
        let cols = ((rect.width() / 2.0) as usize).max(2);
        let xs: Vec<f32> = (0..=cols)
            .map(|c| rect.left() + c as f32 * rect.width() / cols as f32)
            .collect();
        let freqs: Vec<f32> = xs.iter().map(|&x| f_of(rect, x)).collect();
        let phis: Vec<Phi> = freqs.iter().map(|&f| Phi::at(f, fs)).collect();

        self.grid(&painter, rect);

        // analyzer
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.2);
        self.last_frame = now;
        let ana_y = |db: f32| rect.top() + (ANA_TOP - db) / ANA_SPAN * rect.height();
        let (show_pre, show_post) = match self.analyzer_mode {
            AnalyzerMode::Off => (false, false),
            AnalyzerMode::Pre => (true, false),
            AnalyzerMode::Post => (false, true),
            AnalyzerMode::PrePost => (true, true),
        };
        if show_pre {
            self.analyzer.update(false, params, &freqs, dt);
            let ys: Vec<f32> = self
                .analyzer
                .pre
                .db
                .iter()
                .map(|&d| ana_y(d).min(rect.bottom()))
                .collect();
            fill_between(
                &painter,
                &xs,
                &ys,
                |_| rect.bottom(),
                Color32::from_rgba_unmultiplied(0x5a, 0x6e, 0x8c, 70),
            );
        }
        if show_post {
            self.analyzer.update(true, params, &freqs, dt);
            let ys: Vec<f32> = self
                .analyzer
                .post
                .db
                .iter()
                .map(|&d| ana_y(d).min(rect.bottom()))
                .collect();
            fill_between(
                &painter,
                &xs,
                &ys,
                |_| rect.bottom(),
                Color32::from_rgba_unmultiplied(0x8c, 0xb4, 0xe6, 38),
            );
            let line: Vec<Pos2> = xs.iter().zip(&ys).map(|(&x, &y)| pos2(x, y)).collect();
            painter.add(egui::Shape::line(
                line,
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(0xa8, 0xc8, 0xf0, 110)),
            ));
        }

        // per-band curves as heard (dynamic bands at their current gain)
        let hover = resp.hover_pos();
        let hit = hover.and_then(|p| self.hit(rect, bands, p));
        let mut total = vec![0.0f32; xs.len()];
        let y0 = self.y_of(rect, 0.0);
        let clamp = |db: f32| db.clamp(-self.range * 1.6, self.range * 1.6);
        for (i, b) in bands.iter().enumerate() {
            let Some(b) = b else { continue };
            let gain = if b.is_dynamic() {
                params.live_gain(i)
            } else {
                b.gain
            };
            let d = b.design(gain, fs);
            let curve: Vec<f32> = phis.iter().map(|p| d.magnitude_db(p)).collect();
            if b.on {
                for (t, c) in total.iter_mut().zip(&curve) {
                    *t += c;
                }
            }
            let focus = self.selected == Some(i) || hit == Some(i);
            if !b.on && !focus {
                continue;
            }
            let color = if b.on { band_color(i) } else { GREY };
            let ys: Vec<f32> = curve.iter().map(|&c| self.y_of(rect, clamp(c))).collect();
            fill_between(
                &painter,
                &xs,
                &ys,
                |_| y0,
                color.gamma_multiply(if focus { 0.28 } else { 0.10 }),
            );
            if focus {
                let line: Vec<Pos2> = xs.iter().zip(&ys).map(|(&x, &y)| pos2(x, y)).collect();
                painter.add(egui::Shape::line(
                    line,
                    Stroke::new(1.0, color.gamma_multiply(0.8)),
                ));
            }
            // dynamic bands: shade the span the dynamics can move through
            if b.is_dynamic() && focus {
                let lo = b.design(b.gain, fs);
                let hi = b.design(b.gain + b.dynamics.range, fs);
                let ya: Vec<f32> = phis
                    .iter()
                    .map(|p| self.y_of(rect, clamp(lo.magnitude_db(p))))
                    .collect();
                let yb: Vec<f32> = phis
                    .iter()
                    .map(|p| self.y_of(rect, clamp(hi.magnitude_db(p))))
                    .collect();
                fill_between(&painter, &xs, &ya, |c| yb[c], color.gamma_multiply(0.12));
            }
        }
        let line: Vec<Pos2> = xs
            .iter()
            .zip(&total)
            .map(|(&x, &db)| pos2(x, self.y_of(rect, clamp(db))))
            .collect();
        painter.add(egui::Shape::line(
            line,
            Stroke::new(2.0, Color32::from_rgb(0xf4, 0xe6, 0xb8)),
        ));

        self.nodes(ui, &painter, rect, params, bands, hit);
        self.graph_input(ui, rect, resp, params, bands, hit);

        // readout under the pointer
        if let Some(p) = hover
            && hit.is_none()
            && self.drag.is_none()
        {
            let f = f_of(rect, p.x);
            painter.text(
                rect.left_top() + vec2(8.0, 6.0),
                egui::Align2::LEFT_TOP,
                format!(
                    "{}   {}   {}",
                    format_freq(f),
                    note(f),
                    format_db(self.db_of(rect, p.y))
                ),
                egui::FontId::monospace(11.0),
                Color32::from_gray(150),
            );
        }
        if let Some(s) = params.solo() {
            painter.text(
                rect.right_top() + vec2(-8.0, 6.0),
                egui::Align2::RIGHT_TOP,
                format!("SOLO band {}", s + 1),
                egui::FontId::proportional(12.0),
                AMBER,
            );
        }
        if bands.iter().all(Option::is_none) {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "double-click to add a band",
                egui::FontId::proportional(14.0),
                Color32::from_gray(110),
            );
        }
    }

    fn grid(&self, painter: &egui::Painter, rect: Rect) {
        let grid = Stroke::new(1.0, Color32::from_gray(38));
        let label = Color32::from_gray(100);
        let font = egui::FontId::proportional(10.0);
        for f in [
            20.0, 30.0, 40.0, 50.0, 60.0, 80.0, 100.0, 200.0, 300.0, 400.0, 500.0, 600.0, 800.0,
            1000.0, 2000.0, 3000.0, 4000.0, 5000.0, 6000.0, 8000.0, 10_000.0, 20_000.0,
        ] {
            let x = x_of(rect, f);
            let major = [
                20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10_000.0, 20_000.0,
            ]
            .contains(&f);
            painter.line_segment(
                [pos2(x, rect.top()), pos2(x, rect.bottom())],
                if major {
                    grid
                } else {
                    Stroke::new(1.0, Color32::from_gray(29))
                },
            );
            if major {
                let text = if f >= 1000.0 {
                    format!("{}k", f / 1000.0)
                } else {
                    format!("{f}")
                };
                // the last label would run off the right edge
                let (at, align) = if x > rect.right() - 30.0 {
                    (x - 3.0, egui::Align2::RIGHT_BOTTOM)
                } else {
                    (x + 3.0, egui::Align2::LEFT_BOTTOM)
                };
                painter.text(
                    pos2(at, rect.bottom() - 3.0),
                    align,
                    text,
                    font.clone(),
                    label,
                );
            }
        }
        let step = match self.range as i32 {
            3 => 1.0,
            6 => 2.0,
            12 => 3.0,
            _ => 6.0,
        };
        let mut db = -self.range;
        while db <= self.range + 0.01 {
            let y = self.y_of(rect, db);
            let stroke = if db == 0.0 {
                Stroke::new(1.0, Color32::from_gray(70))
            } else {
                grid
            };
            painter.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], stroke);
            if db.abs() < self.range {
                painter.text(
                    pos2(rect.right() - 4.0, y - 1.0),
                    egui::Align2::RIGHT_BOTTOM,
                    if db == 0.0 {
                        "0".to_string()
                    } else {
                        format!("{db:+}")
                    },
                    font.clone(),
                    label,
                );
            }
            db += step;
        }
        // analyzer scale on the left
        if self.analyzer_mode != AnalyzerMode::Off {
            for db in [-20.0, -40.0, -60.0, -80.0] {
                let y = rect.top() + (ANA_TOP - db) / ANA_SPAN * rect.height();
                painter.text(
                    pos2(rect.left() + 4.0, y),
                    egui::Align2::LEFT_CENTER,
                    format!("{db}"),
                    font.clone(),
                    Color32::from_gray(70),
                );
            }
        }
    }

    fn hit(&self, rect: Rect, bands: &Slots, p: Pos2) -> Option<usize> {
        // selected first, so overlapping nodes stay grabbable
        let order = self.selected.into_iter().chain(0..MAX_BANDS);
        let mut best = None;
        let mut best_d = (NODE_RADIUS + 4.0).powi(2);
        for i in order {
            if let Some(b) = &bands[i] {
                let d = self.node_pos(rect, b).distance_sq(p);
                if d < best_d {
                    best_d = d;
                    best = Some(i);
                }
            }
        }
        best
    }

    fn nodes(
        &self,
        ui: &Ui,
        painter: &egui::Painter,
        rect: Rect,
        params: &EqParams,
        bands: &Slots,
        hit: Option<usize>,
    ) {
        for (i, b) in bands.iter().enumerate() {
            let Some(b) = b else { continue };
            let c = self.node_pos(rect, b);
            let color = if b.on { band_color(i) } else { GREY };
            if b.is_dynamic() {
                let live = pos2(
                    c.x,
                    self.y_of(rect, params.live_gain(i).clamp(-self.range, self.range)),
                );
                painter.line_segment([c, live], Stroke::new(2.0, color.gamma_multiply(0.7)));
                painter.circle_stroke(live, 4.0, Stroke::new(1.5, color));
            }
            let focus = self.selected == Some(i) || hit == Some(i);
            let r = if focus {
                NODE_RADIUS + 1.5
            } else {
                NODE_RADIUS
            };
            painter.circle(
                c,
                r,
                color.gamma_multiply(if focus { 1.0 } else { 0.85 }),
                Stroke::new(
                    if self.selected == Some(i) { 2.0 } else { 1.0 },
                    if params.solo() == Some(i) {
                        AMBER
                    } else {
                        Color32::WHITE
                    },
                ),
            );
            painter.text(
                c,
                egui::Align2::CENTER_CENTER,
                format!("{}", i + 1),
                egui::FontId::proportional(10.0),
                Color32::BLACK,
            );
            if hit == Some(i) || self.drag.as_ref().is_some_and(|d| d.band == i) {
                let mut text = format!(
                    "{}\n{}  {}",
                    b.shape.name(),
                    format_freq(b.freq),
                    note(b.freq)
                );
                if b.shape.has_gain() {
                    text += &format!("\n{}", format_db(b.gain));
                }
                if b.shape != Shape::LowCut && b.shape != Shape::HighCut || b.slope > 6 {
                    text += &format!("   Q {:.2}", b.q);
                }
                if b.shape.has_slope() {
                    text += &format!("   {} dB/oct", b.slope);
                }
                let below = c.y < rect.top() + 70.0;
                let at = c + vec2(0.0, if below { 16.0 } else { -16.0 });
                let galley = ui.painter().layout_no_wrap(
                    text,
                    egui::FontId::proportional(11.0),
                    Color32::from_gray(230),
                );
                let size = galley.size() + vec2(10.0, 6.0);
                let mut r = Rect::from_min_size(
                    pos2(
                        at.x - size.x / 2.0,
                        if below { at.y } else { at.y - size.y },
                    ),
                    size,
                );
                r = r.translate(vec2(
                    (rect.left() - r.left()).max(0.0) + (rect.right() - r.right()).min(0.0),
                    0.0,
                ));
                painter.rect_filled(r, 4.0, Color32::from_black_alpha(200));
                painter.galley(r.min + vec2(5.0, 3.0), galley, Color32::WHITE);
            }
        }
    }

    fn graph_input(
        &mut self,
        ui: &Ui,
        rect: Rect,
        resp: &egui::Response,
        params: &EqParams,
        bands: &Slots,
        hit: Option<usize>,
    ) {
        let fine = ui.input(|i| i.modifiers.shift);
        // hit-test where the button went down: by the time egui calls it a
        // drag, a fast pointer may already have left the node
        if resp.drag_started()
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
            && let Some(i) = self.hit(rect, bands, origin)
            && let Some(b) = bands[i]
        {
            self.selected = Some(i);
            self.drag = Some(Drag {
                band: i,
                start: b,
                origin,
                fine,
            });
        }
        if let Some(mut d) = self.drag.take() {
            if let Some(p) = resp.interact_pointer_pos()
                && resp.dragged()
            {
                // toggling fine mode rebases, so the node doesn't jump
                if fine != d.fine
                    && let Some(now) = bands[d.band]
                {
                    (d.start, d.origin, d.fine) = (now, p, fine);
                }
                let scale = if fine { 0.1 } else { 1.0 };
                let delta = (p - d.origin) * scale;
                let mut b = d.start;
                let start = self.node_pos(rect, &d.start);
                b.freq = f_of(rect, start.x + delta.x);
                if b.shape.has_gain() {
                    b.gain = (self.db_of(rect, start.y + delta.y) * 10.0).round() / 10.0;
                } else {
                    b.q = d.start.q * 2f32.powf(-delta.y / 60.0);
                }
                params.set_band(d.band, Some(b));
            }
            let released = resp.drag_stopped() || !ui.input(|i| i.pointer.any_down());
            if !released {
                self.drag = Some(d);
            }
        }
        if resp.hovered()
            && let Some(i) = hit
            && let Some(mut b) = bands[i]
        {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                b.q *= 2f32.powf(scroll / if fine { 600.0 } else { 120.0 });
                params.set_band(i, Some(b));
                ui.ctx().input_mut(|i| i.smooth_scroll_delta.y = 0.0);
            }
        }
        if resp.double_clicked() {
            match hit {
                Some(i) => {
                    if let Some(mut b) = bands[i] {
                        b.on = !b.on;
                        params.set_band(i, Some(b));
                    }
                }
                None => {
                    if let Some(p) = resp.interact_pointer_pos() {
                        self.add_at(rect, params, p);
                    }
                }
            }
        } else if resp.clicked() {
            self.selected = hit;
        }
        if resp.secondary_clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            self.menu = Some(match hit {
                Some(i) => {
                    self.selected = Some(i);
                    MenuTarget::Band(i)
                }
                None => MenuTarget::Empty {
                    freq: f_of(rect, p.x),
                    gain: (self.db_of(rect, p.y) * 10.0).round() / 10.0,
                },
            });
        }
        resp.context_menu(|ui| self.context_menu(ui, params));
    }

    fn add_at(&mut self, rect: Rect, params: &EqParams, p: Pos2) {
        let freq = f_of(rect, p.x);
        let edge = rect.width() * 0.05;
        let band = if p.x < rect.left() + edge {
            Band::new(Shape::LowCut, freq.max(20.0), 0.0)
        } else if p.x > rect.right() - edge {
            Band::new(Shape::HighCut, freq.min(18_000.0), 0.0)
        } else {
            Band::new(
                Shape::Bell,
                freq,
                (self.db_of(rect, p.y) * 10.0).round() / 10.0,
            )
        };
        if let Some(i) = params.add_band(band) {
            self.selected = Some(i);
        }
    }

    fn context_menu(&mut self, ui: &mut Ui, params: &EqParams) {
        match self.menu {
            Some(MenuTarget::Band(i)) => {
                let Some(mut b) = params.band(i) else {
                    ui.close();
                    return;
                };
                ui.label(
                    RichText::new(format!("Band {}", i + 1))
                        .color(band_color(i))
                        .strong(),
                );
                ui.separator();
                for s in Shape::ALL {
                    if ui.radio(b.shape == s, s.name()).clicked() {
                        b.shape = s;
                    }
                }
                if b.shape.has_slope() {
                    ui.separator();
                    ui.menu_button(format!("Slope: {} dB/oct", b.slope), |ui| {
                        for s in SLOPES {
                            if ui.radio(b.slope == s, format!("{s} dB/oct")).clicked() {
                                b.slope = s;
                            }
                        }
                    });
                }
                ui.separator();
                ui.checkbox(&mut b.on, "Enabled");
                if b.shape.has_gain() {
                    ui.checkbox(&mut b.dynamics.on, "Dynamic");
                }
                let mut solo = params.solo() == Some(i);
                if ui.checkbox(&mut solo, "Solo").changed() {
                    params.set_solo(solo.then_some(i));
                }
                if Some(b) != params.band(i) {
                    params.set_band(i, Some(b));
                }
                ui.separator();
                if ui.button("Delete band").clicked() {
                    params.set_band(i, None);
                    ui.close();
                }
            }
            Some(MenuTarget::Empty { freq, gain }) => {
                ui.label(RichText::new(format!("Add band at {}", format_freq(freq))).weak());
                for s in Shape::ALL {
                    if ui.button(s.name()).clicked() {
                        let g = if s.has_gain() { gain } else { 0.0 };
                        if let Some(i) = params.add_band(Band::new(s, freq, g)) {
                            self.selected = Some(i);
                        }
                        ui.close();
                    }
                }
            }
            None => {
                ui.close();
            }
        }
    }

    fn band_strip(&mut self, ui: &mut Ui, params: &EqParams) {
        let Some(i) = self.selected else {
            ui.allocate_ui(vec2(ui.available_width(), 80.0), |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        RichText::new(
                            "select a band, or double-click the graph to add one \
                             (right-click for all shapes)",
                        )
                        .weak(),
                    );
                });
            });
            return;
        };
        let Some(mut b) = params.band(i) else { return };
        let before = b;
        let color = band_color(i);
        ui.horizontal_wrapped(|ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let used: Vec<usize> = (0..MAX_BANDS).filter(|&k| params.band(k).is_some()).collect();
                    let at = used.iter().position(|&k| k == i).unwrap_or(0);
                    if ui.small_button("◀").clicked() {
                        self.selected = Some(used[(at + used.len() - 1) % used.len()]);
                    }
                    ui.label(RichText::new(format!("Band {}", i + 1)).color(color).strong());
                    if ui.small_button("▶").clicked() {
                        self.selected = Some(used[(at + 1) % used.len()]);
                    }
                });
                egui::ComboBox::from_id_salt("eq_shape")
                    .width(100.0)
                    .selected_text(b.shape.name())
                    .show_ui(ui, |ui| {
                        for s in Shape::ALL {
                            ui.selectable_value(&mut b.shape, s, s.name());
                        }
                    });
                ui.add_enabled_ui(b.shape.has_slope(), |ui| {
                    egui::ComboBox::from_id_salt("eq_slope")
                        .width(100.0)
                        .selected_text(format!("{} dB/oct", b.slope))
                        .show_ui(ui, |ui| {
                            for s in SLOPES {
                                ui.selectable_value(&mut b.slope, s, format!("{s} dB/oct"));
                            }
                        });
                });
            });
            ui.separator();
            ui.add(
                Knob::new(&mut b.freq, filter::FREQ_MIN..=filter::FREQ_MAX, "FREQ", &format_freq)
                    .log()
                    .default(1000.0)
                    .color(color),
            );
            ui.add_enabled(
                b.shape.has_gain(),
                Knob::new(&mut b.gain, -filter::GAIN_MAX..=filter::GAIN_MAX, "GAIN", &format_db)
                    .default(0.0)
                    .color(color),
            );
            let q_fmt = |q: f32| format!("{q:.2}");
            ui.add_enabled(
                !(b.shape.has_slope() && b.slope == 6),
                Knob::new(&mut b.q, filter::Q_MIN..=filter::Q_MAX, "Q", &q_fmt)
                    .log()
                    .default(b.shape.default_q())
                    .color(color),
            );
            ui.separator();

            // dynamics
            ui.vertical(|ui| {
                ui.add_enabled_ui(b.shape.has_gain(), |ui| {
                    let (text, fill) = if b.dynamics.on { ("DYNAMIC", AMBER) } else { ("DYNAMIC", GREY) };
                    if ui
                        .add(egui::Button::new(RichText::new(text).small().strong().color(Color32::WHITE)).fill(fill))
                        .on_hover_text("move this band's gain with the level in its region (de-essing, taming resonances)")
                        .clicked()
                    {
                        b.dynamics.on = !b.dynamics.on;
                    }
                });
                if b.is_dynamic() {
                    level_bar(ui, params.level(i), b.dynamics.threshold);
                }
            });
            if b.is_dynamic() {
                let ms = |v: f32| format!("{v:.0} ms");
                let d = &mut b.dynamics;
                ui.add(Knob::new(&mut d.threshold, -80.0..=0.0, "THRESH", &|v| format!("{v:.1} dB")).default(-30.0).color(AMBER));
                ui.add(Knob::new(&mut d.range, -filter::GAIN_MAX..=filter::GAIN_MAX, "RANGE", &format_db).default(-6.0).color(AMBER));
                ui.add(Knob::new(&mut d.attack_ms, 0.5..=500.0, "ATTACK", &|v| format!("{v:.1} ms")).log().default(5.0).color(AMBER));
                ui.add(Knob::new(&mut d.release_ms, 5.0..=2000.0, "RELEASE", &ms).log().default(100.0).color(AMBER));
            }
            ui.separator();
            ui.vertical(|ui| {
                if super::on_off(ui, b.on) {
                    b.on = !b.on;
                }
                let solo = params.solo() == Some(i);
                let fill = if solo { AMBER } else { GREY };
                if ui
                    .add(egui::Button::new(RichText::new("SOLO").small().strong().color(Color32::WHITE)).fill(fill))
                    .on_hover_text("listen to this band's region only")
                    .clicked()
                {
                    params.set_solo((!solo).then_some(i));
                }
                if ui.small_button("Delete").clicked() {
                    params.set_band(i, None);
                    self.selected = None;
                }
            });
        });
        if b != before && params.band(i).is_some() {
            params.set_band(i, Some(b));
        }
    }

    fn bottom_bar(&mut self, ui: &mut Ui, params: &EqParams) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Analyzer");
            egui::ComboBox::from_id_salt("eq_analyzer")
                .width(90.0)
                .selected_text(analyzer_name(self.analyzer_mode))
                .show_ui(ui, |ui| {
                    for m in [
                        AnalyzerMode::Off,
                        AnalyzerMode::Pre,
                        AnalyzerMode::Post,
                        AnalyzerMode::PrePost,
                    ] {
                        ui.selectable_value(&mut self.analyzer_mode, m, analyzer_name(m));
                    }
                });
            ui.label("Range");
            egui::ComboBox::from_id_salt("eq_range")
                .width(70.0)
                .selected_text(format!("±{} dB", self.range))
                .show_ui(ui, |ui| {
                    for r in RANGES {
                        ui.selectable_value(&mut self.range, r, format!("±{r} dB"));
                    }
                });
            ui.separator();
            ui.label("Output");
            let mut out = params.output_gain.get();
            if ui
                .add(
                    egui::DragValue::new(&mut out)
                        .range(-30.0..=30.0)
                        .speed(0.1)
                        .suffix(" dB")
                        .max_decimals(1),
                )
                .on_hover_text("output gain of the eq")
                .changed()
            {
                params.output_gain.set(out);
            }
            ui.separator();
            if ui
                .add_enabled(!self.undo.is_empty(), egui::Button::new("Undo"))
                .on_hover_text("undo (ctrl/cmd+z)")
                .clicked()
            {
                self.undo_redo(params, true);
            }
            if ui
                .add_enabled(!self.redo.is_empty(), egui::Button::new("Redo"))
                .on_hover_text("redo (shift+ctrl/cmd+z)")
                .clicked()
            {
                self.undo_redo(params, false);
            }
            if ui
                .button("Reset")
                .on_hover_text("back to a single low cut")
                .clicked()
            {
                for k in 0..MAX_BANDS {
                    params.set_band(k, None);
                }
                for b in default_bands() {
                    params.add_band(b);
                }
                params.output_gain.set(0.0);
                self.selected = None;
            }
            let n = (0..MAX_BANDS).filter(|&k| params.band(k).is_some()).count();
            ui.label(RichText::new(format!("{n}/{MAX_BANDS} bands")).weak());
        });
    }

    fn keys(&mut self, ui: &Ui, params: &EqParams) {
        // leave keys alone while a text field has focus
        if ui.ctx().memory(|m| m.focused().is_some()) {
            return;
        }
        let (undo, redo, delete) = ui.input(|i| {
            let cmd = i.modifiers.command;
            (
                cmd && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                cmd && i.modifiers.shift && i.key_pressed(egui::Key::Z),
                i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
            )
        });
        if undo {
            self.undo_redo(params, true);
        } else if redo {
            self.undo_redo(params, false);
        }
        if delete && let Some(i) = self.selected.take() {
            params.set_band(i, None);
        }
    }

    fn undo_redo(&mut self, params: &EqParams, undo: bool) {
        let (from, to) = if undo {
            (&mut self.undo, &mut self.redo)
        } else {
            (&mut self.redo, &mut self.undo)
        };
        let Some(state) = from.pop() else { return };
        to.push(std::array::from_fn(|i| params.band(i)));
        for (i, b) in state.iter().enumerate() {
            if params.band(i) != *b {
                params.set_band(i, *b);
            }
        }
        self.gesture = None;
    }
}

fn analyzer_name(m: AnalyzerMode) -> &'static str {
    match m {
        AnalyzerMode::Off => "Off",
        AnalyzerMode::Pre => "Pre",
        AnalyzerMode::Post => "Post",
        AnalyzerMode::PrePost => "Pre + Post",
    }
}

/// detector level with the threshold marked
fn level_bar(ui: &mut Ui, level: f32, threshold: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(72.0, 8.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, Color32::from_gray(30));
    let x = |db: f32| rect.left() + ((db + 80.0) / 80.0).clamp(0.0, 1.0) * rect.width();
    let over = level > threshold;
    painter.rect_filled(
        Rect::from_min_max(rect.min, pos2(x(level), rect.bottom())),
        2.0,
        if over { AMBER } else { GREEN },
    );
    painter.line_segment(
        [
            pos2(x(threshold), rect.top() - 2.0),
            pos2(x(threshold), rect.bottom() + 2.0),
        ],
        Stroke::new(2.0, Color32::WHITE),
    );
}

/// fills the area between two curves sampled at `xs`
fn fill_between(
    painter: &egui::Painter,
    xs: &[f32],
    ys: &[f32],
    base: impl Fn(usize) -> f32,
    color: Color32,
) {
    let mut mesh = Mesh::default();
    for (c, (&x, &y)) in xs.iter().zip(ys).enumerate() {
        mesh.colored_vertex(pos2(x, y), color);
        mesh.colored_vertex(pos2(x, base(c)), color);
        if c > 0 {
            let v = (c * 2) as u32;
            mesh.add_triangle(v - 2, v - 1, v);
            mesh.add_triangle(v - 1, v + 1, v);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// drives the view headlessly with synthetic input
    struct Harness {
        ctx: egui::Context,
        view: EqView,
        params: Arc<EqParams>,
        time: f64,
    }

    impl Harness {
        fn new(bands: &[Band]) -> Self {
            let mut h = Self {
                ctx: egui::Context::default(),
                view: EqView::new(&EqConfig::default()),
                params: Arc::new(EqParams::new(bands, 0.0)),
                time: 0.0,
            };
            h.frame(vec![]);
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            self.time += 1.0 / 60.0;
            let (view, params) = (&mut self.view, &self.params);
            let mut out = self.ctx.run_ui(input, |ui| {
                view.show(ui, params);
            });
            out.textures_delta.clear();
        }

        fn at(&self, freq: f32, db: f32) -> Pos2 {
            let r = self.view.graph_rect;
            pos2(x_of(r, freq), self.view.y_of(r, db))
        }

        fn button(&mut self, p: Pos2, pressed: bool) {
            self.frame(vec![egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }]);
        }

        fn click(&mut self, p: Pos2) {
            self.frame(vec![egui::Event::PointerMoved(p)]);
            self.button(p, true);
            self.button(p, false);
        }

        fn drag(&mut self, from: Pos2, to: Pos2) {
            self.frame(vec![egui::Event::PointerMoved(from)]);
            self.button(from, true);
            for k in 1..=10 {
                let p = from + (to - from) * (k as f32 / 10.0);
                self.frame(vec![egui::Event::PointerMoved(p)]);
            }
            self.button(to, false);
            self.frame(vec![]);
        }

        fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
            let event = egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            };
            self.frame(vec![egui::Event::ModifiersChanged(modifiers), event]);
            self.frame(vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
        }
    }

    #[test]
    fn graph_editing() {
        let mut h = Harness::new(&[]);
        // double-click adds a bell where the pointer is
        let p = h.at(1000.0, 6.0);
        h.click(p);
        h.click(p);
        let b = h.params.band(0).expect("band added");
        assert_eq!(b.shape, Shape::Bell);
        assert!((b.freq / 1000.0 - 1.0).abs() < 0.03, "{}", b.freq);
        assert!((b.gain - 6.0).abs() < 0.3, "{}", b.gain);

        // dragging the node moves it
        let (from, to) = (h.at(b.freq, b.gain), h.at(4000.0, -3.0));
        h.drag(from, to);
        let b = h.params.band(0).unwrap();
        assert!((b.freq / 4000.0 - 1.0).abs() < 0.05, "{}", b.freq);
        assert!((b.gain + 3.0).abs() < 0.4, "{}", b.gain);

        // scrolling over it changes q
        let q = b.q;
        let over = h.at(b.freq, b.gain);
        h.frame(vec![
            egui::Event::PointerMoved(over),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, 40.0),
                modifiers: Default::default(),
                phase: egui::TouchPhase::Move,
            },
        ]);
        for _ in 0..30 {
            h.frame(vec![]);
        }
        assert!(h.params.band(0).unwrap().q > q * 1.05);

        // delete removes the selected band, undo brings it back
        h.key(egui::Key::Delete, egui::Modifiers::NONE);
        assert!(h.params.band(0).is_none());
        h.frame(vec![]);
        h.key(egui::Key::Z, egui::Modifiers::COMMAND);
        let back = h.params.band(0).expect("undo restores the band");
        assert!((back.freq / 4000.0 - 1.0).abs() < 0.05);

        // double-click near the left edge makes a low cut
        let r = h.view.graph_rect;
        let edge = pos2(r.left() + 5.0, r.center().y);
        h.click(edge);
        h.click(edge);
        assert!(h.params.snapshot().iter().any(|b| b.shape == Shape::LowCut));
    }

    #[test]
    fn note_names() {
        assert_eq!(note(440.0), "A4 +0");
        assert_eq!(note(261.63), "C4 +0");
        assert_eq!(note(1000.0), "B5 +21");
    }
}
