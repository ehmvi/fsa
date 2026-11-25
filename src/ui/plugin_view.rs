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

    fn matches(&self, query: &str) -> bool {
        let name = self.name().to_lowercase();
        let vendor = self.vendor().to_lowercase();
        let format = self.format().to_lowercase();
        query
            .split_whitespace()
            .all(|term| name.contains(term) || vendor.contains(term) || format == term)
    }
}

const MODAL_W: f32 = 460.0;
const LIST_H: f32 = 320.0;
const ROW_H: f32 = 26.0;

#[derive(Default)]
pub struct Browser {
    pub open: bool,
    was_open: bool,
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
        if !self.open {
            self.was_open = false;
            return None;
        }
        let just_opened = !self.was_open;
        self.was_open = true;
        let mut picked = None;
        let mut close = false;
        let frame = super::theme::card()
            .inner_margin(egui::Margin::same(20))
            .shadow(ctx.global_style().visuals.window_shadow);
        let modal = egui::Modal::new(egui::Id::new("add_plugin"))
            .backdrop_color(egui::Color32::from_black_alpha(150))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(MODAL_W);
                ui.set_max_width(MODAL_W);
                // same header as the panels: title over the amber rule
                ui.horizontal(|ui| {
                    ui.vertical(|ui| super::title(ui, "Add plugin", false));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if super::close_button(ui)
                            .on_hover_text("close (esc)")
                            .clicked()
                        {
                            close = true;
                        }
                    });
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    // search field and rescan share one height and the full width
                    let rescan_w = 76.0;
                    let search = ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .hint_text("Search by name, vendor or format…")
                            .margin(egui::Margin::symmetric(10, 4))
                            .min_size(egui::vec2(0.0, super::CONTROL_H))
                            .desired_width(ui.available_width() - rescan_w),
                    );
                    if just_opened {
                        search.request_focus();
                    }
                    if super::ghost(ui, "Rescan", true, false)
                        .on_hover_text("look for newly installed plugins")
                        .clicked()
                    {
                        self.plugins = None;
                        self.rescan_lv2 = true;
                    }
                });
                ui.add_space(4.0);
                let filter = self.filter.trim().to_lowercase();
                let filtered: Vec<&Choice> = self
                    .plugins
                    .as_deref()
                    .map(|list| list.iter().filter(|p| p.matches(&filter)).collect())
                    .unwrap_or_default();
                let total = self.plugins.as_ref().map_or(0, Vec::len);

                ui.horizontal(|ui| {
                    let caption = RichText::new("SEARCH PATHS")
                        .font(super::theme::mono(10.5))
                        .color(super::theme::MUTED);
                    egui::CollapsingHeader::new(caption)
                        .id_salt("search_paths")
                        .show(ui, |ui| {
                            for (format, paths) in [
                                ("CLAP", search_paths()),
                                ("LV2", crate::plugins::lv2::search_paths()),
                            ] {
                                ui.label(RichText::new(format).small().strong());
                                for p in paths {
                                    let text =
                                        RichText::new(p.display().to_string()).small().monospace();
                                    ui.label(if p.is_dir() { text } else { text.weak() });
                                }
                            }
                        });
                    if self.plugins.is_some() && total > 0 {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            let label = if filter.is_empty() {
                                format!("{total} plugins")
                            } else {
                                format!("{} / {total}", filtered.len())
                            };
                            ui.label(
                                RichText::new(label)
                                    .font(super::theme::mono(10.5))
                                    .color(super::theme::SUBTLE),
                            );
                        });
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
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("No plugins found")
                                .size(14.0)
                                .color(super::theme::INK),
                        );
                        ui.label(
                            RichText::new("Install CLAP or LV2 plugins, then rescan.")
                                .color(super::theme::MUTED),
                        );
                    });
                    ui.add_space(24.0);
                    return;
                }
                if filtered.is_empty() {
                    ui.allocate_ui_with_layout(
                        egui::vec2(MODAL_W, LIST_H),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            ui.set_min_size(egui::vec2(MODAL_W, LIST_H));
                            ui.add_space(LIST_H * 0.32);
                            ui.label(
                                RichText::new("No matching plugins")
                                    .size(14.0)
                                    .color(super::theme::INK),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new("Try a different name, vendor, or format.")
                                    .color(super::theme::MUTED),
                            );
                            ui.add_space(12.0);
                            if super::ghost(ui, "Clear search", true, false).clicked() {
                                self.filter.clear();
                            }
                        },
                    );
                    return;
                }
                // fixed height keeps the modal anchored while the user filters
                egui::ScrollArea::vertical()
                    .id_salt("plugin_browser_list")
                    .auto_shrink([false, false])
                    .min_scrolled_height(LIST_H)
                    .max_height(LIST_H)
                    .show_rows(ui, ROW_H, filtered.len(), |ui, rows| {
                        let row_w = ui.available_width();
                        for i in rows {
                            let p = filtered[i];
                            let (rect, resp) = ui.allocate_exact_size(
                                egui::vec2(row_w, ROW_H),
                                egui::Sense::click(),
                            );
                            if resp.hovered() {
                                ui.painter()
                                    .rect_filled(rect, 5.0, super::theme::SURFACE_HI);
                            }
                            let mut row_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(rect.shrink2(egui::vec2(6.0, 0.0)))
                                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                            );
                            row_ui.add_sized(
                                [36.0, ROW_H],
                                egui::Label::new(
                                    RichText::new(p.format())
                                        .font(super::theme::mono(10.0))
                                        .color(super::theme::SUBTLE),
                                )
                                .selectable(false),
                            );
                            let mut job = egui::text::LayoutJob::default();
                            job.wrap.max_rows = 1;
                            job.append(
                                p.name(),
                                0.0,
                                egui::TextFormat::simple(
                                    egui::FontId::proportional(13.0),
                                    super::theme::INK,
                                ),
                            );
                            if !p.vendor().is_empty() {
                                job.append(
                                    &format!("  ·  {}", p.vendor()),
                                    0.0,
                                    egui::TextFormat::simple(
                                        egui::FontId::proportional(12.0),
                                        super::theme::MUTED,
                                    ),
                                );
                            }
                            row_ui.add(egui::Label::new(job).truncate().selectable(false));
                            let resp = resp.on_hover_text(p.details());
                            if resp.clicked() {
                                picked = Some(p.clone());
                            }
                        }
                    });
            });
        if close || modal.should_close() || picked.is_some() {
            self.open = false;
            self.was_open = false;
        }
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn fake_clap(name: &str, vendor: &str) -> Choice {
        Choice::Clap(PluginInfo {
            path: PathBuf::from("/tmp/test.clap"),
            id: format!("org.test.{name}"),
            name: name.to_string(),
            vendor: vendor.to_string(),
        })
    }

    fn fake_lv2(name: &str, author: &str) -> Choice {
        Choice::Lv2(Lv2Info {
            uri: format!("urn:test:{name}"),
            name: name.to_string(),
            author: author.to_string(),
        })
    }

    #[test]
    fn browser_multi_token_filter() {
        let clap = fake_clap("LSP Gate Mono", "LSP CLAP");
        let lv2 = fake_lv2("LSP Gate Mono", "LSP LV2");
        assert!(clap.matches(""));
        assert!(clap.matches("gate"));
        assert!(clap.matches("clap gate"));
        assert!(!clap.matches("lv2 gate"));
        assert!(lv2.matches("lv2 gate"));
        assert!(lv2.matches("lsp mono"));
        assert!(!lv2.matches("compressor"));
    }

    #[test]
    fn browser_modal_size_is_stable_across_filters() {
        let ctx = egui::Context::default();
        super::super::theme::install(&ctx);
        let mut browser = Browser {
            open: true,
            plugins: Some(
                (0..100)
                    .map(|i| {
                        fake_clap(
                            &format!("Very Long Plugin Name Number {i} With Extra Technical Suffixes For Testing Width"),
                            "Some Very Long Vendor Name Incorporated",
                        )
                    })
                    .collect(),
            ),
            ..Default::default()
        };

        let run_frame = |b: &mut Browser| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1040.0, 700.0),
                )),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                let _ = b.show(ui.ctx());
            });
            out.textures_delta.clear();
            ctx.memory(|m| m.area_rect(egui::Id::new("add_plugin")))
                .expect("modal area rect")
        };

        // settle two frames with 100 long-named plugins
        run_frame(&mut browser);
        let full = run_frame(&mut browser);
        // width must not stretch past MODAL_W + frame margins (40px) + small slack
        assert!(
            full.width() <= MODAL_W + 48.0,
            "modal stretched too wide: {}",
            full.width()
        );
        // height must fit header + search + full 320px list (not collapsed to 2 rows)
        assert!(
            full.height() >= LIST_H + 100.0,
            "modal list collapsed too short: {}",
            full.height()
        );

        // narrow to 1 match: modal position and size should stay stable
        browser.filter = "number 42".into();
        run_frame(&mut browser);
        let one = run_frame(&mut browser);
        assert!(
            (one.width() - full.width()).abs() < 2.0,
            "width jumped: {} -> {}",
            full.width(),
            one.width()
        );
        assert!(
            (one.height() - full.height()).abs() < 2.0,
            "height jumped: {} -> {}",
            full.height(),
            one.height()
        );

        // narrow to 0 matches: modal stays stable
        browser.filter = "zzzz-no-match".into();
        run_frame(&mut browser);
        let none = run_frame(&mut browser);
        assert!(
            (none.width() - full.width()).abs() < 2.0,
            "width jumped on empty: {} -> {}",
            full.width(),
            none.width()
        );
        assert!(
            (none.height() - full.height()).abs() < 2.0,
            "height jumped on empty: {} -> {}",
            full.height(),
            none.height()
        );
    }
}
