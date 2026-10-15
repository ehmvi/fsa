//! persistent settings in ~/.config/fsa/config.toml.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

use crate::dsp::denoise::DenoiseSettings;
use crate::dsp::eq::{Band, Dynamics, MAX_BANDS, default_bands};
use crate::dsp::filter::Shape;
use crate::dsp::gate::GateSettings;
use crate::shared::InputChannel;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// node.name of the capture source; none = system default
    pub input: Option<String>,
    /// macos: device the processed signal is played into (none = blackhole)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    pub input_channel: InputChannel,
    pub bypass: bool,
    /// processing order, top to bottom
    pub chain: Vec<ChainEntry>,
    pub eq: EqConfig,
    pub gate: GateConfig,
    pub denoise: DenoiseConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            input: None,
            output: None,
            input_channel: InputChannel::Left,
            bypass: false,
            chain: vec![
                ChainEntry::Denoise { enabled: true },
                ChainEntry::Eq { enabled: true },
                ChainEntry::Gate { enabled: true },
            ],
            eq: EqConfig::default(),
            gate: GateConfig::default(),
            denoise: DenoiseConfig::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ChainEntry {
    Denoise {
        enabled: bool,
    },
    Eq {
        enabled: bool,
    },
    Gate {
        enabled: bool,
    },
    Clap {
        enabled: bool,
        /// display name, also used if the plugin goes missing
        name: String,
        id: String,
        path: PathBuf,
        /// plugin state blob, base64
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state: Option<String>,
    },
    Lv2 {
        enabled: bool,
        /// display name, also used if the plugin goes missing
        name: String,
        uri: String,
        /// control input values by port symbol
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        controls: BTreeMap<String, f32>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BandConfig {
    pub on: bool,
    /// missing in configs from the fixed 5-band eq, see `EqConfig::to_bands`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<Shape>,
    #[serde(default = "default_slope")]
    pub slope: u8,
    pub freq: f32,
    pub gain: f32,
    pub q: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamics: Option<DynamicsConfig>,
}

fn default_slope() -> u8 {
    12
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DynamicsConfig {
    pub on: bool,
    pub threshold: f32,
    pub range: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
}

impl From<Band> for BandConfig {
    fn from(b: Band) -> Self {
        let d = b.dynamics;
        Self {
            on: b.on,
            shape: Some(b.shape),
            slope: b.slope,
            freq: b.freq,
            gain: b.gain,
            q: b.q,
            // only written once dynamics were touched
            dynamics: (d != Dynamics::default()).then_some(DynamicsConfig {
                on: d.on,
                threshold: d.threshold,
                range: d.range,
                attack_ms: d.attack_ms,
                release_ms: d.release_ms,
            }),
        }
    }
}

impl BandConfig {
    fn to_band(self, shape: Shape) -> Band {
        Band {
            on: self.on,
            shape,
            slope: self.slope,
            freq: self.freq,
            gain: self.gain,
            q: self.q,
            dynamics: self.dynamics.map_or_else(Dynamics::default, |d| Dynamics {
                on: d.on,
                threshold: d.threshold,
                range: d.range,
                attack_ms: d.attack_ms,
                release_ms: d.release_ms,
            }),
        }
        .clamped()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalyzerMode {
    Off,
    Pre,
    Post,
    #[default]
    PrePost,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EqConfig {
    pub output_gain: f32,
    pub analyzer: AnalyzerMode,
    /// +- db shown by the response graph
    pub display_range: f32,
    pub bands: Vec<BandConfig>,
}

impl Default for EqConfig {
    fn default() -> Self {
        Self {
            output_gain: 0.0,
            analyzer: AnalyzerMode::default(),
            display_range: 12.0,
            bands: default_bands().into_iter().map(Into::into).collect(),
        }
    }
}

impl EqConfig {
    pub fn to_bands(&self) -> Vec<Band> {
        // the old eq had fixed bands: high-pass, low shelf, two peaks, high
        // shelf. flat ones did nothing and are dropped.
        const LEGACY: [Shape; 5] = [
            Shape::LowCut,
            Shape::LowShelf,
            Shape::Bell,
            Shape::Bell,
            Shape::HighShelf,
        ];
        self.bands
            .iter()
            .enumerate()
            .filter_map(|(i, b)| match b.shape {
                Some(shape) => Some(b.to_band(shape)),
                None => {
                    let shape = LEGACY.get(i).copied().unwrap_or_default();
                    (shape == Shape::LowCut || b.gain.abs() >= 0.05).then(|| b.to_band(shape))
                }
            })
            .take(MAX_BANDS)
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GateConfig {
    pub threshold_db: f32,
    pub range_db: f32,
    pub attack_ms: f32,
    pub hold_ms: f32,
    pub release_ms: f32,
}

impl Default for GateConfig {
    fn default() -> Self {
        GateSettings::default().into()
    }
}

impl From<GateSettings> for GateConfig {
    fn from(s: GateSettings) -> Self {
        Self {
            threshold_db: s.threshold_db,
            range_db: s.range_db,
            attack_ms: s.attack_ms,
            hold_ms: s.hold_ms,
            release_ms: s.release_ms,
        }
    }
}

impl From<GateConfig> for GateSettings {
    fn from(c: GateConfig) -> Self {
        Self {
            threshold_db: c.threshold_db.clamp(-100.0, 0.0),
            range_db: c.range_db.clamp(-100.0, 0.0),
            attack_ms: c.attack_ms.clamp(0.1, 50.0),
            hold_ms: c.hold_ms.clamp(0.0, 500.0),
            release_ms: c.release_ms.clamp(5.0, 1000.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DenoiseConfig {
    pub strength: f32,
    pub vad_threshold: f32,
}

impl Default for DenoiseConfig {
    fn default() -> Self {
        DenoiseSettings::default().into()
    }
}

impl From<DenoiseSettings> for DenoiseConfig {
    fn from(s: DenoiseSettings) -> Self {
        Self {
            strength: s.strength,
            vad_threshold: s.vad_threshold,
        }
    }
}

impl From<DenoiseConfig> for DenoiseSettings {
    fn from(c: DenoiseConfig) -> Self {
        Self {
            strength: c.strength.clamp(0.0, 1.0),
            vad_threshold: c.vad_threshold.clamp(0.0, 1.0),
        }
    }
}

pub fn default_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("fsa").join("config.toml")
}

impl Config {
    /// loads the config, falling back to defaults. a broken file is kept
    /// next to the original as config.toml.bad instead of being overwritten.
    pub fn load(path: &Path) -> Self {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                log::warn!("reading {}: {e}, using defaults", path.display());
                return Self::default();
            }
        };
        match toml::from_str::<Config>(&text) {
            Ok(mut c) => {
                c.normalize();
                c
            }
            Err(e) => {
                let bad = path.with_extension("toml.bad");
                log::warn!(
                    "{} is invalid ({e}); moved to {}, using defaults",
                    path.display(),
                    bad.display()
                );
                let _ = std::fs::rename(path, &bad);
                Self::default()
            }
        }
    }

    /// makes sure the built-ins appear exactly once
    fn normalize(&mut self) {
        let mut seen_eq = false;
        let mut seen_gate = false;
        let mut seen_denoise = false;
        self.chain.retain(|e| match e {
            ChainEntry::Denoise { .. } => !std::mem::replace(&mut seen_denoise, true),
            ChainEntry::Eq { .. } => !std::mem::replace(&mut seen_eq, true),
            ChainEntry::Gate { .. } => !std::mem::replace(&mut seen_gate, true),
            ChainEntry::Clap { .. } | ChainEntry::Lv2 { .. } => true,
        });
        if !seen_eq {
            self.chain.insert(0, ChainEntry::Eq { enabled: true });
        }
        if !seen_gate {
            self.chain.push(ChainEntry::Gate { enabled: true });
        }
        if !seen_denoise {
            // added by an update: don't change how an existing setup sounds
            self.chain.insert(0, ChainEntry::Denoise { enabled: false });
        }
    }

    /// writes atomically (temp file + rename)
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let dir = path.parent().context("config path has no parent")?;
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let text = toml::to_string_pretty(self)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path).with_context(|| format!("renaming to {}", path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut c = Config {
            input: Some("alsa_input.usb-Yamaha_Steinberg_UR22C".into()),
            output: Some("coreaudio:BlackHole2ch_UID".into()),
            input_channel: InputChannel::Right,
            ..Default::default()
        };
        c.chain.reverse();
        c.chain.push(ChainEntry::Clap {
            enabled: false,
            name: "Some Comp".into(),
            id: "org.example.comp".into(),
            path: "/run/current-system/sw/lib/clap/comp.clap".into(),
            state: Some("AAEC".into()),
        });
        c.eq.bands.push(Band::new(Shape::Bell, 3000.0, -3.5).into());
        let mut dynamic = Band::new(Shape::HighShelf, 6000.0, 2.0);
        dynamic.dynamics.on = true;
        c.eq.bands.push(dynamic.into());
        let text = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn missing_builtins_are_restored() {
        let mut c: Config =
            toml::from_str("bypass = true\nchain = [{ kind = \"gate\", enabled = false }]")
                .unwrap();
        c.normalize();
        assert!(c.bypass);
        assert_eq!(
            c.chain,
            [
                // a config from before denoise existed gets it, but switched off
                ChainEntry::Denoise { enabled: false },
                ChainEntry::Eq { enabled: true },
                ChainEntry::Gate { enabled: false },
            ]
        );
    }

    #[test]
    fn old_fixed_band_eq_is_migrated() {
        let c: Config = toml::from_str(
            r#"
            [eq]
            bands = [
                { on = true, freq = 90.0, gain = 0.0, q = 0.8 },
                { on = true, freq = 150.0, gain = 0.0, q = 0.707 },
                { on = false, freq = 400.0, gain = -2.0, q = 1.0 },
                { on = true, freq = 3000.0, gain = 0.0, q = 1.0 },
                { on = true, freq = 8000.0, gain = 1.5, q = 0.707 },
            ]
            "#,
        )
        .unwrap();
        let bands = c.eq.to_bands();
        let shapes: Vec<_> = bands.iter().map(|b| (b.shape, b.on, b.freq)).collect();
        assert_eq!(
            shapes,
            [
                (Shape::LowCut, true, 90.0),
                (Shape::Bell, false, 400.0),
                (Shape::HighShelf, true, 8000.0),
            ]
        );
        assert_eq!(bands[0].slope, 12);
        assert_eq!(c.eq.display_range, 12.0);
    }

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!("fsa-test-{}", std::process::id()));
        let path = dir.join("config.toml");
        let mut c = Config::default();
        c.gate.threshold_db = -42.0;
        c.save(&path).unwrap();
        assert_eq!(Config::load(&path), c);
        std::fs::write(&path, "chain = 5").unwrap();
        assert_eq!(Config::load(&path), Config::default());
        assert!(path.with_extension("toml.bad").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
