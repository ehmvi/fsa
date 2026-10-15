//! the gui: chain list on the left, the selected item's panel in the middle,
//! meters on the right.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::prelude::BASE64_STANDARD;
use egui::{Color32, RichText};

use crate::audio::{Backend, Ctrl, Device, HAS_OUTPUT_PICKER, SAMPLE_RATE};
use crate::config::{ChainEntry, Config, EqConfig};
use crate::dsp::denoise::{Denoise, DenoiseParams};
use crate::dsp::eq::{Eq, EqParams};
use crate::dsp::gate::{Gate, GateParams};
use crate::engine::{Command, EngineHandle, Passthrough, Slot, SlotState, Unit};
use crate::plugins::clap::{ClapPlugin, ClapUnit};
use crate::plugins::lv2::{Lv2Host, Lv2Info, Lv2Plugin};
use crate::plugins::scan::PluginInfo;
use crate::shared::{InputChannel, Shared};
use crate::ui::meter::Meter;
use crate::ui::plugin_view::{Browser, Choice};
use crate::ui::{self, AMBER, GREEN};

/// how long settings must stay unchanged before they are written to disk
const SAVE_DEBOUNCE: Duration = Duration::from_millis(800);
/// how long to wait on exit for the audio thread to hand plugins back
const TEARDOWN_TIMEOUT: Duration = Duration::from_millis(500);

pub enum ItemKind {
    Denoise,
    Eq,
    Gate,
    Plugin {
        plugin: Box<ClapPlugin>,
        /// last saved plugin state, base64; refreshed when the plugin changes
        state: Option<String>,
    },
    Lv2 {
        plugin: Box<Lv2Plugin>,
    },
    /// a configured plugin that failed to load; kept so the config survives
    Missing(ChainEntry),
}

pub struct ChainItem {
    pub id: u32,
    pub kind: ItemKind,
    pub state: Arc<SlotState>,
}

impl ChainItem {
    fn name(&self) -> String {
        match &self.kind {
            ItemKind::Eq => "Equalizer".into(),
            ItemKind::Gate => "Noise gate".into(),
            ItemKind::Denoise => "Noise suppression".into(),
            ItemKind::Plugin { plugin, .. } => plugin.info.name.clone(),
            ItemKind::Lv2 { plugin } => plugin.info.name.clone(),
            ItemKind::Missing(ChainEntry::Clap { name, .. } | ChainEntry::Lv2 { name, .. }) => {
                format!("{name} (missing)")
            }
            ItemKind::Missing(_) => "(missing)".into(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.state.enabled.load(Ordering::Relaxed)
    }

    fn to_entry(&self) -> ChainEntry {
        let enabled = self.enabled();
        match &self.kind {
            ItemKind::Eq => ChainEntry::Eq { enabled },
            ItemKind::Gate => ChainEntry::Gate { enabled },
            ItemKind::Denoise => ChainEntry::Denoise { enabled },
            ItemKind::Plugin { plugin, state } => ChainEntry::Clap {
                enabled,
                name: plugin.info.name.clone(),
                id: plugin.info.id.clone(),
                path: plugin.info.path.clone(),
                state: state.clone(),
            },
            ItemKind::Lv2 { plugin } => ChainEntry::Lv2 {
                enabled,
                name: plugin.info.name.clone(),
                uri: plugin.info.uri.clone(),
                controls: plugin.controls(),
            },
            ItemKind::Missing(entry) => entry.clone(),
        }
    }
}

fn encode_state(plugin: &mut ClapPlugin) -> Option<String> {
    plugin.save_state().map(|b| BASE64_STANDARD.encode(b))
}

/// everything built from the config before the audio thread starts
pub struct Setup {
    pub eq: Arc<EqParams>,
    pub gate: Arc<GateParams>,
    pub denoise: Arc<DenoiseParams>,
    pub chain: Vec<ChainItem>,
    pub slots: Vec<Slot>,
    /// created only if the config has lv2 plugins
    pub lv2: Option<Lv2Host>,
}

impl Setup {
    pub fn from_config(config: &Config, shared: &Shared) -> Self {
        let fs = SAMPLE_RATE as f32;
        shared.set_input_channel(config.input_channel);
        shared.bypass.store(config.bypass, Ordering::Relaxed);
        let eq = Arc::new(EqParams::new(&config.eq.to_bands(), config.eq.output_gain));
        let gate = Arc::new(GateParams::new(&config.gate.into()));
        let denoise = Arc::new(DenoiseParams::new(&config.denoise.into()));
        let mut chain = Vec::new();
        let mut slots = Vec::new();
        let mut lv2: Option<Lv2Host> = None;
        for (id, entry) in config.chain.iter().enumerate() {
            let (kind, enabled, unit): (_, _, Box<dyn Unit>) = match entry {
                ChainEntry::Eq { enabled } => {
                    (ItemKind::Eq, *enabled, Box::new(Eq::new(eq.clone(), fs)))
                }
                ChainEntry::Denoise { enabled } => (
                    ItemKind::Denoise,
                    *enabled,
                    Box::new(Denoise::new(denoise.clone())),
                ),
                ChainEntry::Gate { enabled } => (
                    ItemKind::Gate,
                    *enabled,
                    Box::new(Gate::new(gate.clone(), fs)),
                ),
                ChainEntry::Lv2 {
                    enabled,
                    name,
                    uri,
                    controls,
                } => match lv2.get_or_insert_with(Lv2Host::new).load(uri, controls) {
                    Ok((plugin, unit)) => (
                        ItemKind::Lv2 {
                            plugin: Box::new(plugin),
                        },
                        *enabled,
                        Box::new(unit),
                    ),
                    Err(e) => {
                        log::error!("{name}: {e:#}");
                        (
                            ItemKind::Missing(entry.clone()),
                            false,
                            Box::new(Passthrough),
                        )
                    }
                },
                ChainEntry::Clap {
                    enabled,
                    name,
                    id,
                    path,
                    state,
                } => {
                    let info = PluginInfo {
                        path: path.clone(),
                        id: id.clone(),
                        name: name.clone(),
                        vendor: String::new(),
                    };
                    let bytes = state.as_ref().and_then(|s| BASE64_STANDARD.decode(s).ok());
                    match ClapPlugin::load(&info, bytes.as_deref()) {
                        Ok((plugin, unit)) => (
                            ItemKind::Plugin {
                                plugin: Box::new(plugin),
                                state: state.clone(),
                            },
                            *enabled,
                            Box::new(unit),
                        ),
                        Err(e) => {
                            log::error!("{name}: {e:#}");
                            (
                                ItemKind::Missing(entry.clone()),
                                false,
                                Box::new(Passthrough),
                            )
                        }
                    }
                }
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
            denoise,
            chain,
            slots,
            lv2,
        }
    }
}

enum RowAction {
    Select,
    Move(usize, i32),
    Remove(usize),
    ToggleEditor(usize),
}

/// device picker; returns the new selection when the user picked one
fn device_combo(
    ui: &mut egui::Ui,
    id: &str,
    current: &Option<String>,
    devices: &[Device],
    none_label: &str,
) -> Option<Option<String>> {
    let text = match current {
        None => none_label.to_string(),
        Some(name) => devices
            .iter()
            .find(|d| &d.name == name)
            .map(|d| d.description.clone())
            .unwrap_or_else(|| format!("{name} (missing)")),
    };
    let mut pick = None;
    egui::ComboBox::from_id_salt(id)
        .width(240.0)
        .selected_text(text)
        .show_ui(ui, |ui| {
            if ui.selectable_label(current.is_none(), none_label).clicked() {
                pick = Some(None);
            }
            for d in devices {
                let sel = current.as_deref() == Some(d.name.as_str());
                if ui
                    .selectable_label(sel, &d.description)
                    .on_hover_text(&d.name)
                    .clicked()
                {
                    pick = Some(Some(d.name.clone()));
                }
            }
        });
    pick
}

/// one row of the chain list
fn chain_row(
    ui: &mut egui::Ui,
    item: &ChainItem,
    i: usize,
    len: usize,
    selected: bool,
) -> Option<RowAction> {
    let mut action = None;
    // buttons first (right to left) so the name truncates in the space that's left
    let size = egui::vec2(ui.available_width(), 26.0);
    ui.allocate_ui_with_layout(
        size,
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            if matches!(
                item.kind,
                ItemKind::Plugin { .. } | ItemKind::Lv2 { .. } | ItemKind::Missing(_)
            ) && ui.button("🗙").on_hover_text("remove from chain").clicked()
            {
                action = Some(RowAction::Remove(i));
            }
            if ui
                .add_enabled(i + 1 < len, egui::Button::new("⏷"))
                .on_hover_text("move down")
                .clicked()
            {
                action = Some(RowAction::Move(i, 1));
            }
            if ui
                .add_enabled(i > 0, egui::Button::new("⏶"))
                .on_hover_text("move up")
                .clicked()
            {
                action = Some(RowAction::Move(i, -1));
            }
            if let ItemKind::Plugin { plugin, .. } = &item.kind
                && plugin.has_editor()
            {
                let text = if plugin.editor_open() { "Hide" } else { "Edit" };
                if ui
                    .button(text)
                    .on_hover_text("plugin editor window")
                    .clicked()
                {
                    action = Some(RowAction::ToggleEditor(i));
                }
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let missing = matches!(item.kind, ItemKind::Missing(_));
                if ui
                    .add_enabled_ui(!missing, |ui| ui::on_off(ui, item.enabled()))
                    .inner
                {
                    item.state.enabled.store(!item.enabled(), Ordering::Relaxed);
                }
                let label = RichText::new(item.name()).size(15.0);
                let label = if item.enabled() { label } else { label.weak() };
                let label = if missing { label.color(AMBER) } else { label };
                let resp = ui.add(egui::Button::selectable(selected, label).truncate());
                if resp.on_hover_text(item.name()).clicked() {
                    action = Some(RowAction::Select);
                }
            });
        },
    );
    action
}

pub struct App {
    shared: Arc<Shared>,
    backend: Backend,
    engine: EngineHandle,
    eq: Arc<EqParams>,
    eq_view: ui::eq_view::EqView,
    gate: Arc<GateParams>,
    denoise: Arc<DenoiseParams>,
    chain: Vec<ChainItem>,
    next_id: u32,
    /// plugins removed from the chain, waiting for the audio thread to let go
    removing: Vec<(u32, Box<ClapPlugin>)>,
    /// every removed plugin slot not yet handed back by the audio thread
    awaiting: Vec<u32>,
    lv2: Option<Lv2Host>,
    /// lilv worlds replaced by a rescan, kept alive for the instances they made
    retired_lv2: Vec<Lv2Host>,
    browser: Browser,
    input: Option<String>,
    output: Option<String>,
    selected: Option<u32>,
    error: Option<String>,
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
            denoise,
            chain,
            slots: _,
            lv2,
        } = setup;
        let selected = chain.first().map(|c| c.id);
        let next_id = chain.iter().map(|c| c.id + 1).max().unwrap_or(0);
        let error = chain
            .iter()
            .any(|c| matches!(c.kind, ItemKind::Missing(_)))
            .then(|| "some plugins could not be loaded, see the log".to_string());
        Self {
            shared,
            backend,
            engine,
            eq_view: ui::eq_view::EqView::new(&config.eq),
            eq,
            gate,
            denoise,
            chain,
            next_id,
            removing: Vec::new(),
            awaiting: Vec::new(),
            lv2,
            retired_lv2: Vec::new(),
            browser: Browser::default(),
            input: config.input.clone(),
            output: config.output.clone(),
            selected,
            error,
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
            output: self.output.clone(),
            input_channel: self.shared.input_channel(),
            bypass: self.shared.bypass.load(Ordering::Relaxed),
            chain: self.chain.iter().map(ChainItem::to_entry).collect(),
            eq: EqConfig {
                output_gain: self.eq.output_gain.get(),
                analyzer: self.eq_view.analyzer_mode,
                display_range: self.eq_view.range,
                bands: self.eq.snapshot().into_iter().map(Into::into).collect(),
            },
            gate: self.gate.get().into(),
            denoise: self.denoise.get().into(),
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
        let after = to.checked_sub(1).map(|i| self.chain[i].id);
        self.engine.send(Command::Move { id, after });
    }

    fn add_plugin(&mut self, info: &PluginInfo) {
        match ClapPlugin::load(info, None) {
            Ok((mut plugin, unit)) => {
                let id = self.next_id;
                self.next_id += 1;
                let state = SlotState::new(true);
                let fs = SAMPLE_RATE as f32;
                self.engine.send(Command::Insert(Slot::new(
                    id,
                    Box::new(unit),
                    state.clone(),
                    fs,
                )));
                let saved = encode_state(&mut plugin);
                self.chain.push(ChainItem {
                    id,
                    kind: ItemKind::Plugin {
                        plugin: Box::new(plugin),
                        state: saved,
                    },
                    state,
                });
                self.selected = Some(id);
            }
            Err(e) => self.error = Some(format!("{}: {e:#}", info.name)),
        }
    }

    fn add_lv2(&mut self, info: &Lv2Info) {
        let host = self.lv2.get_or_insert_with(Lv2Host::new);
        match host.load(&info.uri, &Default::default()) {
            Ok((plugin, unit)) => {
                let id = self.next_id;
                self.next_id += 1;
                let state = SlotState::new(true);
                let slot = Slot::new(id, Box::new(unit), state.clone(), SAMPLE_RATE as f32);
                self.engine.send(Command::Insert(slot));
                self.chain.push(ChainItem {
                    id,
                    kind: ItemKind::Lv2 {
                        plugin: Box::new(plugin),
                    },
                    state,
                });
                self.selected = Some(id);
            }
            Err(e) => self.error = Some(format!("{}: {e:#}", info.name)),
        }
    }

    fn remove_item(&mut self, index: usize) {
        let item = self.chain.remove(index);
        self.engine.send(Command::Remove(item.id));
        self.awaiting.push(item.id);
        if let ItemKind::Plugin { mut plugin, .. } = item.kind {
            plugin.close_editor();
            self.removing.push((item.id, plugin));
        }
        if self.selected == Some(item.id) {
            self.selected = self.chain.get(index.saturating_sub(1)).map(|c| c.id);
        }
    }

    /// deactivates plugins whose audio side came back from the engine
    fn collect_garbage(&mut self) {
        for slot in self.engine.collect_garbage() {
            self.awaiting.retain(|id| *id != slot.id);
            // anything but clap (e.g. lv2 instances) is simply dropped here, on the gui thread
            let Some(pos) = self.removing.iter().position(|(id, _)| *id == slot.id) else {
                continue;
            };
            let (_, plugin) = self.removing.swap_remove(pos);
            if let Ok(unit) = slot.unit.into_any().downcast::<ClapUnit>() {
                plugin.deactivate(*unit);
            }
        }
    }

    /// main-thread plugin housekeeping. returns true if an editor is open.
    fn poll_plugins(&mut self) -> bool {
        let mut editor_open = false;
        for item in &mut self.chain {
            if let ItemKind::Plugin { plugin, state } = &mut item.kind {
                if plugin.poll() {
                    *state = encode_state(plugin);
                }
                editor_open |= plugin.editor_open();
            }
        }
        editor_open
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Input");
            let devices = self.shared.devices.lock().unwrap().clone();
            if let Some(p) = device_combo(ui, "input", &self.input, &devices, "System default")
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

            if HAS_OUTPUT_PICKER {
                ui.label("Output").on_hover_text(
                    "the processed signal plays into this device; pick it as the mic in discord",
                );
                let outputs = self.shared.outputs.lock().unwrap().clone();
                if let Some(p) =
                    device_combo(ui, "output", &self.output, &outputs, "BlackHole (auto)")
                    && p != self.output
                {
                    self.output = p.clone();
                    self.backend.send(Ctrl::SetOutput(p));
                }
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
        let mut action = None;
        let len = self.chain.len();
        egui::ScrollArea::vertical()
            .max_height(ui.available_height() - 40.0)
            .show(ui, |ui| {
                for (i, item) in self.chain.iter().enumerate() {
                    let selected = self.selected == Some(item.id);
                    let fill = if selected {
                        ui.visuals().selection.bg_fill.gamma_multiply(0.35)
                    } else {
                        Color32::TRANSPARENT
                    };
                    egui::Frame::group(ui.style())
                        .fill(fill)
                        .inner_margin(6.0)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            if let Some(a) = chain_row(ui, item, i, len, selected) {
                                match a {
                                    RowAction::Select => self.selected = Some(item.id),
                                    other => action = Some(other),
                                }
                            }
                        });
                }
            });
        ui.add_space(6.0);
        if ui.button("➕ Add plugin…").clicked() {
            self.browser.open = true;
        }
        match action {
            Some(RowAction::Move(i, dir)) => self.move_item(i, (i as i32 + dir) as usize),
            Some(RowAction::Remove(i)) => self.remove_item(i),
            Some(RowAction::ToggleEditor(i)) => {
                if let ItemKind::Plugin { plugin, .. } = &mut self.chain[i].kind {
                    if plugin.editor_open() {
                        plugin.close_editor();
                    } else if let Err(e) = plugin.open_editor() {
                        self.error = Some(format!("{}: {e:#}", plugin.info.name));
                    }
                }
            }
            Some(RowAction::Select) | None => {}
        }
    }

    fn selected_panel(&mut self, ui: &mut egui::Ui) {
        let Some(index) = self.chain.iter().position(|c| Some(c.id) == self.selected) else {
            self.eq_view.hidden(&self.eq);
            ui.label("select an item in the chain");
            return;
        };
        let item = &mut self.chain[index];
        if !matches!(item.kind, ItemKind::Eq) {
            self.eq_view.hidden(&self.eq);
        }
        ui.horizontal(|ui| {
            ui.heading(item.name());
            if !item.enabled() {
                ui.label(RichText::new("(off)").weak());
            }
        });
        ui.separator();
        match &mut item.kind {
            ItemKind::Eq => {
                self.eq_view.show(ui, &self.eq);
            }
            ItemKind::Gate => {
                ui::gate_view::show(ui, &self.gate);
            }
            ItemKind::Denoise => {
                ui::denoise_view::show(ui, &self.denoise);
            }
            ItemKind::Plugin { plugin, .. } => {
                if let Err(e) = ui::plugin_view::show(ui, plugin) {
                    self.error = Some(format!("{}: {e:#}", plugin.info.name));
                }
            }
            ItemKind::Lv2 { plugin } => ui::plugin_view::show_lv2(ui, plugin),
            ItemKind::Missing(entry) => {
                ui.label("this plugin could not be loaded and is bypassed.");
                let details = match entry {
                    ChainEntry::Clap { id, path, .. } => format!("{id}\n{}", path.display()),
                    ChainEntry::Lv2 { uri, .. } => uri.clone(),
                    _ => String::new(),
                };
                ui.label(RichText::new(details).monospace().small());
                ui.label("its settings are kept in the config until you remove it.");
            }
        }
    }

    /// hands every plugin back to the main thread and deactivates it
    fn teardown_plugins(&mut self) {
        for i in (0..self.chain.len()).rev() {
            if matches!(
                self.chain[i].kind,
                ItemKind::Plugin { .. } | ItemKind::Lv2 { .. }
            ) {
                self.remove_item(i);
            }
        }
        let deadline = Instant::now() + TEARDOWN_TIMEOUT;
        while !self.awaiting.is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
            self.collect_garbage();
        }
        if !self.awaiting.is_empty() {
            // lv2 instances still on the audio thread must not outlive their world
            // (the lint fires for the non-linux stub, which has nothing to drop)
            #[allow(clippy::forget_non_drop)]
            std::mem::forget(self.lv2.take());
            std::mem::forget(std::mem::take(&mut self.retired_lv2));
            // the audio thread isn't running; clack leaks rather than destroying
            // an instance whose processor is still alive, which is fine at exit
            log::warn!(
                "{} plugin(s) not returned by the audio thread",
                self.awaiting.len()
            );
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.collect_garbage();
        if self.browser.scan_pending() {
            if self.browser.rescan_lv2 {
                // lilv reads bundles once per world; keep the old one for live instances
                self.retired_lv2.extend(self.lv2.take());
            }
            let host = self.lv2.get_or_insert_with(Lv2Host::new);
            self.browser.run_pending_scan(host);
        }
        let editor_open = self.poll_plugins();

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
                if let Some(err) = &self.error {
                    ui.label(
                        RichText::new(err)
                            .small()
                            .color(Color32::from_rgb(0xe0, 0x5a, 0x5a)),
                    );
                    if ui.small_button("🗙").clicked() {
                        self.error = None;
                    }
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
            .default_size(300.0)
            .show(ui, |ui| {
                ui.add_space(6.0);
                self.chain_list(ui);
            });
        egui::CentralPanel::default_margins().show(ui, |ui| self.selected_panel(ui));

        match self.browser.show(ui.ctx()) {
            Some(Choice::Clap(info)) => self.add_plugin(&info),
            Some(Choice::Lv2(info)) => self.add_lv2(&info),
            None => {}
        }

        self.autosave();
        // meters, the gate indicator and plugin timers need regular repaints
        let period = if editor_open { 16 } else { 33 };
        ui.ctx()
            .request_repaint_after(Duration::from_millis(period));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        for item in &mut self.chain {
            if let ItemKind::Plugin { plugin, state } = &mut item.kind {
                *state = encode_state(plugin);
            }
        }
        let snap = self.snapshot();
        self.save(snap);
        self.teardown_plugins();
    }
}
