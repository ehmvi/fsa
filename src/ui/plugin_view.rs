//! clap plugin panel (editor button + generic parameters) and the plugin browser.

use egui::{RichText, Ui};

use crate::plugins::clap::ClapPlugin;
use crate::plugins::lv2::{Lv2Host, Lv2Info, Lv2Plugin};
use crate::plugins::scan::{PluginInfo, search_paths};

pub fn show(ui: &mut Ui, plugin: &mut ClapPlugin) -> anyhow::Result<()> {
    let mut result = Ok(());
    ui.horizontal(|ui| {
        if !plugin.info.vendor.is_empty() {
            ui.label(RichText::new(&plugin.info.vendor).weak());
        }
        let open = plugin.editor_open();
        let button = ui.add_enabled(
            plugin.has_editor(),
            egui::Button::new(if open { "Close editor" } else { "Open editor" }),
        );
        let button = if plugin.has_editor() {
            button
        } else {
            button.on_disabled_hover_text("this plugin has no x11 editor; use the parameters below")
        };
        if button.clicked() {
            if open {
                plugin.close_editor();
            } else {
                result = plugin.open_editor();
            }
        }
    });
    ui.label(
        RichText::new(plugin.info.path.display().to_string())
            .small()
            .weak(),
    );
    ui.separator();

    let count = plugin.params.len();
    if count == 0 {
        ui.label("this plugin exposes no parameters");
        return result;
    }
    let row_height = ui.spacing().interact_size.y;
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show_rows(ui, row_height, count, |ui, rows| {
            for i in rows {
                let p = plugin.params[i].clone();
                let Some(mut value) = plugin.param_value(p.id) else {
                    continue;
                };
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(180.0, row_height),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_width(180.0);
                            ui.add(egui::Label::new(&p.name).truncate());
                        },
                    );
                    let mut slider = egui::Slider::new(&mut value, p.min..=p.max).show_value(false);
                    if p.stepped {
                        slider = slider.step_by(1.0);
                    }
                    let resp = ui.add(slider).on_hover_text("double-click to reset");
                    if resp.double_clicked() {
                        value = p.default;
                    }
                    if resp.changed() || resp.double_clicked() {
                        plugin.set_param(p.id, value);
                    }
                    let text = plugin
                        .param_text(p.id, value)
                        .unwrap_or_else(|| format!("{value:.3}"));
                    ui.label(text);
                });
            }
        });
    result
}

/// a plugin picked in the browser
#[derive(Clone, Debug)]
pub enum Choice {
    Clap(PluginInfo),
    Lv2(Lv2Info),
}

impl Choice {
    fn name(&self) -> &str {
        match self {
            Choice::Clap(p) => &p.name,
            Choice::Lv2(p) => &p.name,
        }
    }

    fn vendor(&self) -> &str {
        match self {
            Choice::Clap(p) => &p.vendor,
            Choice::Lv2(p) => &p.author,
        }
    }

    fn format(&self) -> &'static str {
        match self {
            Choice::Clap(_) => "CLAP",
            Choice::Lv2(_) => "LV2",
        }
    }

    fn details(&self) -> String {
        match self {
            Choice::Clap(p) => format!("{}\n{}", p.id, p.path.display()),
            Choice::Lv2(p) => p.uri.clone(),
        }
    }
}

#[derive(Default)]
pub struct Browser {
    pub open: bool,
    filter: String,
    /// none until scanned
    plugins: Option<Vec<Choice>>,
    /// scan at the start of the next frame (so "scanning…" gets drawn first)
    scan_next: bool,
    /// the user asked for a rescan: lv2 needs a fresh lilv world
    pub rescan_lv2: bool,
}

impl Browser {
    pub fn scan_pending(&self) -> bool {
        self.scan_next
    }

    /// runs a pending scan; blocking, clap entry init belongs on the main thread
    pub fn run_pending_scan(&mut self, lv2: &Lv2Host) {
        if !self.scan_next {
            return;
        }
        self.scan_next = false;
        self.rescan_lv2 = false;
        let mut all: Vec<Choice> = crate::plugins::scan::scan()
            .into_iter()
            .map(Choice::Clap)
            .collect();
        all.extend(lv2.plugins().into_iter().map(Choice::Lv2));
        all.sort_by_key(|c| (c.name().to_lowercase(), c.format()));
        self.plugins = Some(all);
    }

    /// returns the plugin the user picked
    pub fn show(&mut self, ctx: &egui::Context) -> Option<Choice> {
        let mut picked = None;
        let mut open = self.open;
        egui::Window::new("Add plugin")
            .open(&mut open)
            .default_size([460.0, 480.0])
            .collapsible(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .hint_text("filter…")
                            .desired_width(280.0),
                    );
                    if ui.button("Rescan").clicked() {
                        self.plugins = None;
                        self.rescan_lv2 = true;
                    }
                });
                ui.collapsing("search paths", |ui| {
                    for (format, paths) in [
                        ("CLAP", search_paths()),
                        ("LV2", crate::plugins::lv2::search_paths()),
                    ] {
                        ui.label(RichText::new(format).small().strong());
                        for p in paths {
                            let text = RichText::new(p.display().to_string()).small().monospace();
                            ui.label(if p.is_dir() { text } else { text.weak() });
                        }
                    }
                });
                ui.separator();
                let Some(plugins) = &self.plugins else {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("scanning for clap and lv2 plugins…");
                    });
                    self.scan_next = true;
                    ctx.request_repaint();
                    return;
                };
                if plugins.is_empty() {
                    ui.label("no plugins found in the search paths");
                    return;
                }
                let filter = self.filter.to_lowercase();
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        for p in plugins.iter().filter(|p| {
                            filter.is_empty()
                                || p.name().to_lowercase().contains(&filter)
                                || p.vendor().to_lowercase().contains(&filter)
                                || p.format().to_lowercase() == filter
                        }) {
                            ui.horizontal(|ui| {
                                ui.add_sized(
                                    [40.0, 18.0],
                                    egui::Label::new(RichText::new(p.format()).small().weak()),
                                );
                                let text = if p.vendor().is_empty() {
                                    p.name().to_string()
                                } else {
                                    format!("{}  ·  {}", p.name(), p.vendor())
                                };
                                let resp = ui
                                    .add(egui::Button::new(text).frame(false))
                                    .on_hover_text(p.details());
                                if resp.clicked() {
                                    picked = Some(p.clone());
                                }
                            });
                        }
                    });
            });
        self.open = open && picked.is_none();
        picked
    }
}

/// generic controls for an lv2 plugin
pub fn show_lv2(ui: &mut Ui, plugin: &mut Lv2Plugin) {
    if !plugin.info.author.is_empty() {
        ui.label(RichText::new(&plugin.info.author).weak());
    }
    ui.label(RichText::new(&plugin.info.uri).small().weak());
    ui.label(
        RichText::new("lv2 plugin uis aren't hosted; generic controls below")
            .small()
            .weak(),
    );
    ui.separator();
    if plugin.params.is_empty() {
        ui.label("this plugin has no controls");
        return;
    }
    let row_height = ui.spacing().interact_size.y;
    let count = plugin.params.len();
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show_rows(ui, row_height, count, |ui, rows| {
            for i in rows {
                let p = plugin.params[i].clone();
                let mut value = p.value;
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(180.0, row_height),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_width(180.0);
                            ui.add(egui::Label::new(&p.name).truncate())
                                .on_hover_text(&p.symbol);
                        },
                    );
                    if p.toggled {
                        let mut on = value > 0.5 * (p.min + p.max);
                        if ui.checkbox(&mut on, "").changed() {
                            value = if on { p.max } else { p.min };
                        }
                    } else if !p.points.is_empty() {
                        let current = p
                            .points
                            .iter()
                            .find(|(v, _)| (*v - value).abs() < 1e-4)
                            .map_or_else(|| format!("{value}"), |(_, l)| l.clone());
                        egui::ComboBox::from_id_salt(("lv2", i))
                            .selected_text(current)
                            .show_ui(ui, |ui| {
                                for (v, label) in &p.points {
                                    ui.selectable_value(&mut value, *v, label);
                                }
                            });
                    } else {
                        let mut slider =
                            egui::Slider::new(&mut value, p.min..=p.max).logarithmic(p.logarithmic);
                        if p.integer {
                            slider = slider.step_by(1.0);
                        }
                        if ui
                            .add(slider)
                            .on_hover_text("double-click to reset")
                            .double_clicked()
                        {
                            value = p.default;
                        }
                    }
                });
                if value != p.value {
                    plugin.set(i, value);
                }
            }
        });
}
