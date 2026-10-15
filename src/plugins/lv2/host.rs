//! lv2 hosting on top of livi (lilv). linux only (lv2-sys is).

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Context as _, anyhow, bail};
use livi::event::LV2AtomSequence;
use livi::{EmptyPortConnections, Features, FeaturesBuilder, PortIndex, PortType};

use super::{Lv2Info, Lv2Param};
use crate::audio::{MAX_BLOCK, SAMPLE_RATE};
use crate::engine::Unit;

/// control changes queued per plugin between two audio blocks
const CONTROL_QUEUE: usize = 1024;
/// bytes per atom sequence port
const ATOM_CAPACITY: usize = 4096;
const LV2_CORE: &str = "http://lv2plug.in/ns/lv2core#";

/// the lilv world, created lazily (parsing every bundle takes a moment)
pub struct Lv2Host {
    world: livi::World,
    features: Arc<Features>,
}

impl Lv2Host {
    pub fn new() -> Self {
        // instruments need midi input, which a mic chain never has
        let world = livi::World::with_plugin_predicate(|p| !p.is_instrument());
        let features = world.build_features(FeaturesBuilder {
            min_block_length: 1,
            max_block_length: MAX_BLOCK,
        });
        Self { world, features }
    }

    /// effects we can run: at least one audio input and one audio output
    pub fn plugins(&self) -> Vec<Lv2Info> {
        let mut out: Vec<Lv2Info> = self
            .world
            .iter_plugins()
            .filter(|p| {
                let c = p.port_counts();
                c.audio_inputs > 0 && c.audio_outputs > 0
            })
            .map(|p| Lv2Info {
                author: p
                    .raw()
                    .author_name()
                    .and_then(|n| n.as_str().map(str::to_string))
                    .unwrap_or_default(),
                uri: p.uri(),
                name: p.name(),
            })
            .collect();
        out.sort_by_key(|p| (p.name.to_lowercase(), p.author.to_lowercase()));
        out
    }

    /// instantiates a plugin and applies saved control values (by symbol)
    pub fn load(
        &self,
        uri: &str,
        controls: &BTreeMap<String, f32>,
    ) -> anyhow::Result<(Lv2Plugin, Lv2Unit)> {
        let plugin = self
            .world
            .plugin_by_uri(uri)
            .with_context(|| format!("lv2 plugin {uri} not found (or unsupported)"))?;
        let counts = *plugin.port_counts();
        if counts.audio_inputs == 0 || counts.audio_outputs == 0 {
            bail!("{} is not an audio effect", plugin.name());
        }
        // safety: running plugin code is the point of a host
        let mut instance = unsafe { plugin.instantiate(self.features.clone(), SAMPLE_RATE as f64) }
            .map_err(|e| anyhow!("instantiating {uri}: {e:?}"))?;

        let world = self.world.raw();
        let prop = |name: &str| world.new_uri(&format!("{LV2_CORE}{name}"));
        let (toggled, integer, enumeration) =
            (prop("toggled"), prop("integer"), prop("enumeration"));
        let logarithmic = world.new_uri("http://lv2plug.in/ns/ext/port-props#logarithmic");
        let not_on_gui = world.new_uri("http://lv2plug.in/ns/ext/port-props#notOnGUI");

        let mut params = Vec::new();
        for port in plugin.ports_with_type(PortType::ControlInput) {
            let raw = plugin.raw().port_by_index(port.index.0);
            let has = |n: &lilv::node::Node| raw.as_ref().is_some_and(|r| r.has_property(n));
            if has(&not_on_gui) {
                continue;
            }
            let mut points: Vec<(f32, String)> = raw
                .as_ref()
                .map(|r| {
                    r.scale_points()
                        .iter()
                        .filter_map(|sp| {
                            Some((sp.value().as_float()?, sp.label().as_str()?.to_string()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            points.sort_by(|a, b| a.0.total_cmp(&b.0));
            let min = port.min_value.unwrap_or(0.0);
            let max = port.max_value.unwrap_or(1.0).max(min);
            let value = controls
                .get(&port.symbol)
                .copied()
                .unwrap_or(port.default_value)
                .clamp(min, max);
            instance.set_control_input(port.index, value);
            params.push(Lv2Param {
                port: port.index.0,
                symbol: port.symbol.clone(),
                name: port.name.clone(),
                min,
                max,
                default: port.default_value,
                toggled: has(&toggled),
                integer: has(&integer),
                logarithmic: has(&logarithmic) && min > 0.0,
                points: if has(&enumeration) || !points.is_empty() {
                    points
                } else {
                    Vec::new()
                },
                value,
            });
        }

        let (tx, rx) = rtrb::RingBuffer::new(CONTROL_QUEUE);
        let buf = || vec![0.0f32; MAX_BLOCK].into_boxed_slice();
        let atoms = |n: usize| {
            (0..n)
                .map(|_| LV2AtomSequence::new(&self.features, ATOM_CAPACITY))
                .collect()
        };
        let unit = Lv2Unit {
            instance,
            rx,
            audio_in: (0..counts.audio_inputs).map(|_| buf()).collect(),
            audio_out: (0..counts.audio_outputs).map(|_| buf()).collect(),
            atom_in: atoms(counts.atom_sequence_inputs),
            atom_out: atoms(counts.atom_sequence_outputs),
            cv_in: (0..counts.cv_inputs).map(|_| buf()).collect(),
            cv_out: (0..counts.cv_outputs).map(|_| buf()).collect(),
        };
        let author = plugin
            .raw()
            .author_name()
            .and_then(|n| n.as_str().map(str::to_string))
            .unwrap_or_default();
        let info = Lv2Info {
            uri: uri.to_string(),
            name: plugin.name(),
            author,
        };
        log::info!(
            "loaded lv2 {} ({} in / {} out, {} controls)",
            info.name,
            counts.audio_inputs,
            counts.audio_outputs,
            params.len()
        );
        Ok((
            Lv2Plugin {
                info,
                params,
                tx,
                _features: self.features.clone(),
            },
            unit,
        ))
    }
}

/// gui side of an lv2 plugin
pub struct Lv2Plugin {
    pub info: Lv2Info,
    pub params: Vec<Lv2Param>,
    tx: rtrb::Producer<(usize, f32)>,
    /// the instance borrows the urid map etc. from here
    _features: Arc<Features>,
}

impl Lv2Plugin {
    pub fn set(&mut self, param: usize, value: f32) {
        let p = &mut self.params[param];
        p.value = value.clamp(p.min, p.max);
        if self.tx.push((p.port, p.value)).is_err() {
            log::warn!("{}: control queue full", self.info.name);
        }
    }

    /// control values by symbol, for the config
    pub fn controls(&self) -> BTreeMap<String, f32> {
        self.params
            .iter()
            .map(|p| (p.symbol.clone(), p.value))
            .collect()
    }
}

/// audio side of an lv2 plugin
pub struct Lv2Unit {
    instance: livi::Instance,
    rx: rtrb::Consumer<(usize, f32)>,
    audio_in: Vec<Box<[f32]>>,
    audio_out: Vec<Box<[f32]>>,
    atom_in: Vec<LV2AtomSequence>,
    atom_out: Vec<LV2AtomSequence>,
    cv_in: Vec<Box<[f32]>>,
    cv_out: Vec<Box<[f32]>>,
}

impl Unit for Lv2Unit {
    fn process(&mut self, buf: &mut [f32]) {
        let n = buf.len();
        while let Ok((index, value)) = self.rx.pop() {
            self.instance.set_control_input(PortIndex(index), value);
        }
        for ch in &mut self.audio_in {
            ch[..n].copy_from_slice(buf);
        }
        for seq in &mut self.atom_in {
            seq.clear();
        }
        let ports = EmptyPortConnections::new()
            .with_audio_inputs(self.audio_in.iter().map(|b| &b[..n]))
            .with_audio_outputs(self.audio_out.iter_mut().map(|b| &mut b[..n]))
            .with_atom_sequence_inputs(self.atom_in.iter())
            .with_atom_sequence_outputs(self.atom_out.iter_mut())
            .with_cv_inputs(self.cv_in.iter().map(|b| &b[..n]))
            .with_cv_outputs(self.cv_out.iter_mut().map(|b| &mut b[..n]));
        // safety: buffers are sized for MAX_BLOCK and match the port counts
        if unsafe { self.instance.run(n, ports) }.is_ok() {
            crate::plugins::mixdown(&self.audio_out, buf);
        }
    }

    fn reset(&mut self) {
        // lv2 has no realtime-safe reset; the plugin keeps its state
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any + Send> {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Command, Engine, Slot, SlotState};
    use crate::shared::Shared;

    /// hosts the plugin with uri $FSA_TEST_LV2 (found via $LV2_PATH). skipped when unset.
    #[test]
    fn hosts_lv2_from_env() {
        let Ok(uri) = std::env::var("FSA_TEST_LV2") else {
            eprintln!("FSA_TEST_LV2 not set, skipping");
            return;
        };
        let host = Lv2Host::new();
        assert!(
            host.plugins().iter().any(|p| p.uri == uri),
            "{uri} not listed"
        );
        let (mut plugin, unit) = host.load(&uri, &BTreeMap::new()).unwrap();
        assert!(
            !plugin.params.is_empty(),
            "test plugin should have controls"
        );

        let (mut engine, mut handle) =
            Engine::new(Arc::new(Shared::default()), Vec::new(), SAMPLE_RATE as f32);
        handle.send(Command::Insert(Slot::new(
            3,
            Box::new(unit),
            SlotState::new(true),
            SAMPLE_RATE as f32,
        )));
        let input = [0.5f32; 256];
        let mut out = [0.0f32; 256];
        engine.process(&input, &mut out);
        let before = out[255];

        let min = plugin.params[0].min;
        plugin.set(0, min);
        engine.process(&input, &mut out);
        assert_ne!(out[255], before, "control change should affect the output");

        // saved controls are restored by symbol
        let (restored, _unit) = host.load(&uri, &plugin.controls()).unwrap();
        assert_eq!(restored.params[0].value, min);
    }
}
