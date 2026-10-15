//! clap plugin panel (editor button + generic parameters) and the plugin browser.

use egui::{RichText, Ui};

use crate::plugins::clap::ClapPlugin;
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

#[derive(Default)]
pub struct Browser {
    pub open: bool,
    filter: String,
    /// none until scanned
    plugins: Option<Vec<PluginInfo>>,
    /// scan at the start of the next frame (so "scanning…" gets drawn first)
    pub scan_next: bool,
}

impl Browser {
    /// runs a pending scan; blocking, clap entry init belongs on the main thread
    pub fn run_pending_scan(&mut self) {
        if self.scan_next {
            self.scan_next = false;
            self.plugins = Some(crate::plugins::scan::scan());
        }
    }

    /// returns the plugin the user picked
    pub fn show(&mut self, ctx: &egui::Context) -> Option<PluginInfo> {
        let mut picked = None;
        let mut open = self.open;
        egui::Window::new("Add plugin")
            .open(&mut open)
            .default_size([420.0, 460.0])
            .collapsible(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .hint_text("filter…")
                            .desired_width(260.0),
                    );
                    if ui.button("Rescan").clicked() {
                        self.plugins = None;
                    }
                });
                ui.collapsing("search paths", |ui| {
                    for p in search_paths() {
                        let text = RichText::new(p.display().to_string()).small().monospace();
                        ui.label(if p.is_dir() { text } else { text.weak() });
                    }
                });
                ui.separator();
                let Some(plugins) = &self.plugins else {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("scanning for clap plugins…");
                    });
                    self.scan_next = true;
                    ctx.request_repaint();
                    return;
                };
                if plugins.is_empty() {
                    ui.label("no clap plugins found in the search paths");
                    return;
                }
                let filter = self.filter.to_lowercase();
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        for p in plugins.iter().filter(|p| {
                            filter.is_empty()
                                || p.name.to_lowercase().contains(&filter)
                                || p.vendor.to_lowercase().contains(&filter)
                        }) {
                            let text = if p.vendor.is_empty() {
                                p.name.clone()
                            } else {
                                format!("{}  ·  {}", p.name, p.vendor)
                            };
                            let resp = ui
                                .add(egui::Button::new(text).frame(false))
                                .on_hover_text(format!("{}\n{}", p.id, p.path.display()));
                            if resp.clicked() {
                                picked = Some(p.clone());
                            }
                        }
                    });
            });
        self.open = open && picked.is_none();
        picked
    }
}
