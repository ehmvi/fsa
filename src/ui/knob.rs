//! rotary knob: drag up/down (shift for fine), scroll, double-click to
//! reset.

use std::ops::RangeInclusive;

use egui::{Color32, Response, Sense, Stroke, Ui, Widget, pos2, vec2};

use super::theme::{self, INK, LINE, MUTED, SIGNAL, SURFACE, SURFACE_HI};

const START: f32 = std::f32::consts::PI * 0.75;
const SWEEP: f32 = std::f32::consts::PI * 1.5;

fn focus_key() -> egui::Id {
    egui::Id::new("fsa_focused_knob")
}

/// true when a widget other than a rotary knob holds keyboard focus (e.g. a
/// text field or DragValue editor), so letter/delete shortcuts should yield
pub fn other_widget_focused(ctx: &egui::Context) -> bool {
    ctx.memory(|m| m.focused())
        .is_some_and(|id| ctx.data(|d| d.get_temp::<egui::Id>(focus_key())) != Some(id))
}

pub struct Knob<'a> {
    value: &'a mut f32,
    range: RangeInclusive<f32>,
    label: &'a str,
    default: f32,
    log: bool,
    color: Color32,
    radius: f32,
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
            color: SIGNAL,
            radius: 17.0,
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

    /// outer radius of the dial
    pub fn radius(mut self, r: f32) -> Self {
        self.radius = r;
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
        let r = self.radius;
        let width = (2.0 * r + 22.0).max(58.0);
        let (rect, mut resp) =
            ui.allocate_exact_size(vec2(width, 2.0 * r + 40.0), Sense::click_and_drag());
        let enabled = ui.is_enabled();
        let mut t = self.t_of(*self.value);
        if enabled {
            if resp.clicked() || resp.drag_started() {
                resp.request_focus();
                ui.data_mut(|d| d.insert_temp(focus_key(), resp.id));
            }
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
            if resp.has_focus() {
                ui.data_mut(|d| d.insert_temp(focus_key(), resp.id));
                let (inc, dec, home, esc) = ui.input(|i| {
                    (
                        i.key_pressed(egui::Key::ArrowUp) || i.key_pressed(egui::Key::ArrowRight),
                        i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::ArrowLeft),
                        i.key_pressed(egui::Key::Home),
                        i.key_pressed(egui::Key::Escape),
                    )
                });
                let step = if fine { 0.004 } else { 0.02 };
                if esc {
                    resp.surrender_focus();
                } else if inc {
                    t = (t + step).clamp(0.0, 1.0);
                    *self.value = self.value_at(t);
                    resp.mark_changed();
                } else if dec {
                    t = (t - step).clamp(0.0, 1.0);
                    *self.value = self.value_at(t);
                    resp.mark_changed();
                } else if home {
                    *self.value = self.default;
                    t = self.t_of(self.default);
                    resp.mark_changed();
                }
            }
            if resp.double_clicked() {
                *self.value = self.default;
                t = self.t_of(self.default);
                resp.mark_changed();
            }
        }

        let focused = enabled && resp.has_focus();
        let active = enabled && (resp.hovered() || resp.dragged() || focused);
        let painter = ui.painter_at(rect.expand(4.0));
        let dim = |c: Color32| if enabled { c } else { c.gamma_multiply(0.35) };
        painter.text(
            pos2(rect.center().x, rect.top()),
            egui::Align2::CENTER_TOP,
            self.label,
            theme::mono(10.0),
            dim(if active { INK } else { MUTED }),
        );
        let c = pos2(rect.center().x, rect.top() + 18.0 + r);
        let stroke_w = if r > 20.0 { 4.0 } else { 3.0 };
        let ring = r - stroke_w / 2.0;
        let at = |t: f32| {
            let a = START + SWEEP * t;
            c + vec2(a.cos(), a.sin()) * ring
        };
        let arc = |from: f32, to: f32| -> Vec<egui::Pos2> {
            let n = ((to - from).abs() * r).ceil().max(2.0) as usize;
            (0..=n)
                .map(|i| at(from + (to - from) * i as f32 / n as f32))
                .collect()
        };
        // track with round ends
        let track = dim(theme::BORDER);
        painter.add(egui::Shape::line(
            arc(0.0, 1.0),
            Stroke::new(stroke_w, track),
        ));
        for end in [0.0, 1.0] {
            painter.circle_filled(at(end), stroke_w / 2.0, track);
        }
        // bipolar ranges fill from the centre
        let (lo, hi) = (*self.range.start(), *self.range.end());
        let origin = if lo < 0.0 && hi > 0.0 {
            self.t_of(0.0)
        } else {
            0.0
        };
        let color = dim(self.color);
        if (t - origin).abs() > 0.002 {
            painter.add(egui::Shape::line(
                arc(origin, t),
                Stroke::new(stroke_w, color),
            ));
            for end in [origin, t] {
                painter.circle_filled(at(end), stroke_w / 2.0, color);
            }
        }
        // cap: a soft disc with a hairline rim and a drop shadow
        let cap = r - stroke_w - 3.5;
        painter.circle_filled(c + vec2(0.0, 1.5), cap + 0.5, Color32::from_black_alpha(80));
        painter.circle(
            c,
            cap,
            dim(if active { SURFACE_HI } else { SURFACE }),
            Stroke::new(
                1.0,
                dim(if focused {
                    SIGNAL.gamma_multiply(0.75)
                } else {
                    LINE
                }),
            ),
        );
        let a = START + SWEEP * t;
        let dir = vec2(a.cos(), a.sin());
        painter.line_segment(
            [c + dir * (cap * 0.3), c + dir * (cap - 2.5)],
            Stroke::new(2.0, dim(INK)),
        );
        painter.text(
            pos2(rect.center().x, rect.bottom()),
            egui::Align2::CENTER_BOTTOM,
            (self.format)(*self.value),
            theme::mono(if r > 20.0 { 12.0 } else { 11.0 }),
            dim(if active { self.color } else { INK }),
        );
        resp.on_hover_text("drag, scroll, or arrow keys (shift: fine); double-click/home: reset")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrow_keys_and_home_adjust_focused_knob() {
        let ctx = egui::Context::default();
        let mut value = 50.0f32;
        let fmt = |v: f32| format!("{v:.0}");

        let run = |events: Vec<egui::Event>, v: &mut f32| -> egui::Pos2 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(300.0, 200.0),
                )),
                events,
                ..Default::default()
            };
            let mut center = egui::Pos2::ZERO;
            let mut out = ctx.run_ui(input, |ui| {
                let r = ui.add(Knob::new(v, 0.0..=100.0, "TEST", &fmt).default(25.0));
                center = r.rect.center();
            });
            out.textures_delta.clear();
            center
        };

        let center = run(vec![], &mut value);
        // click to focus
        run(
            vec![
                egui::Event::PointerMoved(center),
                egui::Event::PointerButton {
                    pos: center,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
            &mut value,
        );
        run(
            vec![egui::Event::PointerButton {
                pos: center,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
            &mut value,
        );
        assert!(
            !other_widget_focused(&ctx),
            "focused knob should not block non-text shortcuts"
        );
        // ArrowUp steps +2%
        run(
            vec![egui::Event::Key {
                key: egui::Key::ArrowUp,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
            &mut value,
        );
        assert!((value - 52.0).abs() < 0.01, "value={value}");
        // Home resets to default
        run(
            vec![egui::Event::Key {
                key: egui::Key::Home,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
            &mut value,
        );
        assert!((value - 25.0).abs() < 0.01, "value={value}");
    }
}
