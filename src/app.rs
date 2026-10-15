//! the gui: chain list on the left, the selected item's panel in the middle.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use egui::{Color32, RichText};

use crate::audio::{Backend, Ctrl, SAMPLE_RATE};
use crate::config::{ChainEntry, Config, EqConfig};
use crate::dsp::eq::{Eq, EqParams};
use crate::dsp::gate::{Gate, GateParams};
use crate::engine::{Command, EngineHandle, Slot, SlotState, Unit};
use crate::shared::{InputChannel, Shared};
use crate::ui::meter::Meter;
use crate::ui::{self, AMBER, GREEN};

/// how long settings must stay unchanged before they are written to disk
const SAVE_DEBOUNCE: Duration = Duration::from_millis(800);

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

    fn to_entry(&self) -> ChainEntry {
        let enabled = self.enabled();
        match self.kind {
            ItemKind::Eq => ChainEntry::Eq { enabled },
            ItemKind::Gate => ChainEntry::Gate { enabled },
        }
    }
}

/// everything built from the config before the audio thread starts
pub struct Setup {
    pub eq: Arc<EqParams>,
    pub gate: Arc<GateParams>,
    pub chain: Vec<ChainItem>,
    pub slots: Vec<Slot>,
}

impl Setup {
    pub fn from_config(config: &Config, shared: &Shared) -> Self {
        let fs = SAMPLE_RATE as f32;
        shared.set_input_channel(config.input_channel);
        shared.bypass.store(config.bypass, Ordering::Relaxed);
        let eq = Arc::new(EqParams::new(&config.eq.to_bands()));
        let gate = Arc::new(GateParams::new(&config.gate.into()));
        let mut chain = Vec::new();
        let mut slots = Vec::new();
        for (id, entry) in config.chain.iter().enumerate() {
            let (kind, enabled, unit): (_, _, Box<dyn Unit>) = match *entry {
                ChainEntry::Eq { enabled } => {
                    (ItemKind::Eq, enabled, Box::new(Eq::new(eq.clone(), fs)))
                }
                ChainEntry::Gate { enabled } => (
                    ItemKind::Gate,
                    enabled,
                    Box::new(Gate::new(gate.clone(), fs)),
                ),
            };
            let item = ChainItem {
                id: id as u32,
                kind,
                state: SlotState::new(enabled),
            };
            slots.push(Slot::new(item.id, unit, item.state.clone(), fs));
            chain.push(item);
        }
        Self {
            eq,
            gate,
            chain,
            slots,
        }
    }
}

pub struct App {
    shared: Arc<Shared>,
    backend: Backend,
    engine: EngineHandle,
    eq: Arc<EqParams>,
    gate: Arc<GateParams>,
    chain: Vec<ChainItem>,
    input: Option<String>,
    selected: Option<u32>,
    meter_in: Meter,
    meter_out: Meter,
    config_path: PathBuf,
    saved: Config,
    /// latest unsaved snapshot and when it last changed
    pending: Option<(Config, Instant)>,
}

impl App {
    pub fn new(
        shared: Arc<Shared>,
        backend: Backend,
        engine: EngineHandle,
        setup: Setup,
        config: Config,
        config_path: PathBuf,
    ) -> Self {
        let Setup {
            eq,
            gate,
            chain,
            slots: _,
        } = setup;
        let selected = chain.first().map(|c| c.id);
        Self {
            shared,
            backend,
            engine,
            eq,
            gate,
            chain,
            input: config.input.clone(),
            selected,
            meter_in: Meter::default(),
            meter_out: Meter::default(),
            config_path,
            saved: config,
            pending: None,
        }
    }

    fn snapshot(&self) -> Config {
        Config {
            input: self.input.clone(),
            input_channel: self.shared.input_channel(),
            bypass: self.shared.bypass.load(Ordering::Relaxed),
            chain: self.chain.iter().map(ChainItem::to_entry).collect(),
            eq: EqConfig {
                bands: self.eq.snapshot().iter().map(|&b| b.into()).collect(),
            },
            gate: self.gate.get().into(),
        }
    }

    fn save(&mut self, config: Config) {
        match config.save(&self.config_path) {
            Ok(()) => log::debug!("saved {}", self.config_path.display()),
            Err(e) => log::error!("saving config: {e:#}"),
        }
        self.saved = config;
        self.pending = None;
    }

    /// saves once the settings have been stable for `SAVE_DEBOUNCE`
    fn autosave(&mut self) {
        let snap = self.snapshot();
        let now = Instant::now();
        match &self.pending {
            _ if snap == self.saved => self.pending = None,
            Some((p, since)) if *p == snap => {
                if now.duration_since(*since) >= SAVE_DEBOUNCE {
                    self.save(snap);
                }
            }
            _ => self.pending = Some((snap, now)),
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
        self.meter_in.update(self.shared.in_peak.take());
        self.meter_out.update(self.shared.out_peak.take());
        egui::Panel::right("meters")
            .resizable(false)
            .show(ui, |ui| {
                ui.add_space(6.0);
                let h = (ui.available_height() - 50.0).max(80.0);
                ui.horizontal(|ui| {
                    self.meter_in.show(ui, "IN", h);
                    self.meter_out.show(ui, "OUT", h);
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

        self.autosave();
        // meters and the gate indicator need regular repaints
        ui.ctx().request_repaint_after(Duration::from_millis(33));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let snap = self.snapshot();
        self.save(snap);
    }
}
