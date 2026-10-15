//! rotary knob: drag up/down (shift for fine), scroll, double-click to
//! reset.

use std::ops::RangeInclusive;

use egui::{Color32, Response, Sense, Stroke, Ui, Widget, pos2, vec2};

const START: f32 = std::f32::consts::PI * 0.75;
const SWEEP: f32 = std::f32::consts::PI * 1.5;

pub struct Knob<'a> {
    value: &'a mut f32,
    range: RangeInclusive<f32>,
    label: &'a str,
    default: f32,
    log: bool,
    color: Color32,
    format: &'a dyn Fn(f32) -> String,
}

impl<'a> Knob<'a> {
    pub fn new(
        value: &'a mut f32,
        range: RangeInclusive<f32>,
        label: &'a str,
        format: &'a dyn Fn(f32) -> String,
    ) -> Self {
        let default = *range.start();
        Self {
            value,
            range,
            label,
            default,
            log: false,
            color: Color32::from_rgb(0x6c, 0xb4, 0xe8),
            format,
        }
    }

    pub fn log(mut self) -> Self {
        self.log = true;
        self
    }

    pub fn default(mut self, v: f32) -> Self {
        self.default = v;
        self
    }

    pub fn color(mut self, c: Color32) -> Self {
        self.color = c;
        self
    }

    fn t_of(&self, v: f32) -> f32 {
        let (lo, hi) = (*self.range.start(), *self.range.end());
        let t = if self.log {
            (v / lo).ln() / (hi / lo).ln()
        } else {
            (v - lo) / (hi - lo)
        };
        t.clamp(0.0, 1.0)
    }

    fn value_at(&self, t: f32) -> f32 {
        let (lo, hi) = (*self.range.start(), *self.range.end());
        if self.log {
            lo * (hi / lo).powf(t)
        } else {
            lo + (hi - lo) * t
        }
    }
}

impl Widget for Knob<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (rect, mut resp) = ui.allocate_exact_size(vec2(50.0, 68.0), Sense::click_and_drag());
        let enabled = ui.is_enabled();
        let mut t = self.t_of(*self.value);
        if enabled {
            let fine = ui.input(|i| i.modifiers.shift);
            if resp.dragged() {
                let speed = if fine { 0.0008 } else { 0.005 };
                t = (t - resp.drag_delta().y * speed + resp.drag_delta().x * speed).clamp(0.0, 1.0);
                *self.value = self.value_at(t);
                resp.mark_changed();
            }
            if resp.hovered() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    t = (t + scroll * if fine { 0.0002 } else { 0.001 }).clamp(0.0, 1.0);
                    *self.value = self.value_at(t);
                    ui.input_mut(|i| i.smooth_scroll_delta.y = 0.0);
                    resp.mark_changed();
                }
            }
            if resp.double_clicked() {
                *self.value = self.default;
                t = self.t_of(self.default);
                resp.mark_changed();
            }
        }

        let painter = ui.painter_at(rect.expand(2.0));
        let dim = |c: Color32| if enabled { c } else { c.gamma_multiply(0.35) };
        let text = dim(Color32::from_gray(170));
        painter.text(
            pos2(rect.center().x, rect.top()),
            egui::Align2::CENTER_TOP,
            self.label,
            egui::FontId::proportional(10.0),
            text,
        );
        let c = pos2(rect.center().x, rect.top() + 32.0);
        let r = 15.0;
        let arc = |from: f32, to: f32| -> Vec<egui::Pos2> {
            let n = ((to - from).abs() * 12.0).ceil().max(2.0) as usize;
            (0..=n)
                .map(|i| {
                    let a = START + SWEEP * (from + (to - from) * i as f32 / n as f32);
                    c + vec2(a.cos(), a.sin()) * r
                })
                .collect()
        };
        painter.add(egui::Shape::line(
            arc(0.0, 1.0),
            Stroke::new(3.0, Color32::from_gray(48)),
        ));
        // bipolar ranges fill from the centre
        let (lo, hi) = (*self.range.start(), *self.range.end());
        let origin = if lo < 0.0 && hi > 0.0 {
            self.t_of(0.0)
        } else {
            0.0
        };
        if (t - origin).abs() > 0.002 {
            painter.add(egui::Shape::line(
                arc(origin, t),
                Stroke::new(3.0, dim(self.color)),
            ));
        }
        painter.circle_filled(c, r - 4.0, dim(Color32::from_gray(58)));
        let a = START + SWEEP * t;
        let dir = vec2(a.cos(), a.sin());
        painter.line_segment(
            [c + dir * 3.0, c + dir * (r - 5.0)],
            Stroke::new(2.0, dim(Color32::from_gray(225))),
        );
        painter.text(
            pos2(rect.center().x, rect.bottom()),
            egui::Align2::CENTER_BOTTOM,
            (self.format)(*self.value),
            egui::FontId::proportional(11.0),
            dim(Color32::from_gray(220)),
        );
        resp.on_hover_text("drag or scroll (shift: fine), double-click: reset")
    }
}
