//! the gui: chain list on the left, the selected item's panel in the middle.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use egui::{Color32, RichText};

use crate::audio::{Backend, Ctrl};
use crate::dsp::eq::EqParams;
use crate::dsp::gate::GateParams;
use crate::engine::{Command, EngineHandle, SlotState};
use crate::shared::{InputChannel, Shared};
use crate::ui::{self, AMBER, GREEN};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Eq,
    Gate,
}

pub struct ChainItem {
    pub id: u32,
    pub kind: ItemKind,
    pub state: Arc<SlotState>,
}

impl ChainItem {
    fn name(&self) -> &str {
        match self.kind {
            ItemKind::Eq => "Equalizer",
            ItemKind::Gate => "Noise gate",
        }
    }

    pub fn enabled(&self) -> bool {
        self.state.enabled.load(Ordering::Relaxed)
    }
}

pub struct App {
    pub shared: Arc<Shared>,
    pub backend: Backend,
    pub engine: EngineHandle,
    pub eq: Arc<EqParams>,
    pub gate: Arc<GateParams>,
    pub chain: Vec<ChainItem>,
    pub input: Option<String>,
    selected: Option<u32>,
}

impl App {
    pub fn new(
        shared: Arc<Shared>,
        backend: Backend,
        engine: EngineHandle,
        eq: Arc<EqParams>,
        gate: Arc<GateParams>,
        chain: Vec<ChainItem>,
        input: Option<String>,
    ) -> Self {
        let selected = chain.first().map(|c| c.id);
        Self {
            shared,
            backend,
            engine,
            eq,
            gate,
            chain,
            input,
            selected,
        }
    }

    fn move_item(&mut self, from: usize, to: usize) {
        let item = self.chain.remove(from);
        let id = item.id;
        self.chain.insert(to, item);
        self.engine.send(Command::Move { id, to });
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Input");
            let devices = self.shared.devices.lock().unwrap().clone();
            let current = match &self.input {
                None => "System default".to_string(),
                Some(name) => devices
                    .iter()
                    .find(|d| &d.name == name)
                    .map(|d| d.description.clone())
                    .unwrap_or_else(|| format!("{name} (missing)")),
            };
            let mut pick = None;
            egui::ComboBox::from_id_salt("input")
                .width(260.0)
                .selected_text(current)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(self.input.is_none(), "System default")
                        .clicked()
                    {
                        pick = Some(None);
                    }
                    for d in &devices {
                        let sel = self.input.as_deref() == Some(d.name.as_str());
                        if ui
                            .selectable_label(sel, &d.description)
                            .on_hover_text(&d.name)
                            .clicked()
                        {
                            pick = Some(Some(d.name.clone()));
                        }
                    }
                });
            if let Some(p) = pick
                && p != self.input
            {
                self.input = p.clone();
                self.backend.send(Ctrl::SetInput(p));
            }

            let mut ch = self.shared.input_channel();
            egui::ComboBox::from_id_salt("channel")
                .width(70.0)
                .selected_text(match ch {
                    InputChannel::Left => "Left",
                    InputChannel::Right => "Right",
                    InputChannel::Mix => "L+R",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut ch, InputChannel::Left, "Left");
                    ui.selectable_value(&mut ch, InputChannel::Right, "Right");
                    ui.selectable_value(&mut ch, InputChannel::Mix, "L+R");
                })
                .response
                .on_hover_text("which input channel carries the mic");
            if ch != self.shared.input_channel() {
                self.shared.set_input_channel(ch);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let bypass = self.shared.bypass.load(Ordering::Relaxed);
                let (text, fill) = if bypass {
                    ("BYPASSED", AMBER)
                } else {
                    ("ACTIVE", GREEN)
                };
                let b = egui::Button::new(RichText::new(text).strong().color(Color32::WHITE))
                    .fill(fill)
                    .min_size(egui::vec2(96.0, 26.0));
                if ui
                    .add(b)
                    .on_hover_text("global bypass: output = raw input")
                    .clicked()
                {
                    self.shared.bypass.store(!bypass, Ordering::Relaxed);
                }
            });
        });
    }

    fn chain_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("Chain");
        ui.label(
            RichText::new("processing order, top to bottom")
                .small()
                .weak(),
        );
        ui.add_space(6.0);
        let mut action: Option<(usize, i32)> = None;
        let len = self.chain.len();
        for (i, item) in self.chain.iter().enumerate() {
            let selected = self.selected == Some(item.id);
            let frame = egui::Frame::group(ui.style())
                .fill(if selected {
                    ui.visuals().selection.bg_fill.gamma_multiply(0.35)
                } else {
                    Color32::TRANSPARENT
                })
                .inner_margin(6.0);
            frame.show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    if ui::on_off(ui, item.enabled()) {
                        item.state.enabled.store(!item.enabled(), Ordering::Relaxed);
                    }
                    let label = RichText::new(item.name()).size(15.0);
                    let label = if item.enabled() { label } else { label.weak() };
                    if ui.selectable_label(selected, label).clicked() {
                        self.selected = Some(item.id);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(i + 1 < len, egui::Button::new("⏷"))
                            .on_hover_text("move down")
                            .clicked()
                        {
                            action = Some((i, 1));
                        }
                        if ui
                            .add_enabled(i > 0, egui::Button::new("⏶"))
                            .on_hover_text("move up")
                            .clicked()
                        {
                            action = Some((i, -1));
                        }
                    });
                });
            });
        }
        if let Some((i, dir)) = action {
            self.move_item(i, (i as i32 + dir) as usize);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        drop(self.engine.collect_garbage());

        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(4.0);
            self.top_bar(ui);
            ui.add_space(4.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(self.shared.status.lock().unwrap().as_str()).small());
                let xruns = self.shared.xruns.load(Ordering::Relaxed);
                if xruns > 0 {
                    ui.label(
                        RichText::new(format!("· {xruns} underruns"))
                            .small()
                            .color(AMBER),
                    );
                }
            });
        });
        egui::Panel::left("chain")
            .resizable(true)
            .default_size(260.0)
            .show(ui, |ui| {
                ui.add_space(6.0);
                self.chain_list(ui);
            });
        egui::CentralPanel::default_margins().show(ui, |ui| {
            let Some(item) = self.chain.iter().find(|c| Some(c.id) == self.selected) else {
                ui.label("select an item in the chain");
                return;
            };
            let (kind, name, enabled) = (item.kind, item.name().to_string(), item.enabled());
            ui.horizontal(|ui| {
                ui.heading(name);
                if !enabled {
                    ui.label(RichText::new("(off)").weak());
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| match kind {
                ItemKind::Eq => {
                    ui::eq_view::show(ui, &self.eq);
                }
                ItemKind::Gate => {
                    ui::gate_view::show(ui, &self.gate);
                }
            });
        });

        // meters and the gate indicator need regular repaints
        ui.ctx().request_repaint_after(Duration::from_millis(33));
    }
}
