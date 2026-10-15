//! persistent settings in ~/.config/fsa/config.toml.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

use crate::dsp::denoise::DenoiseSettings;
use crate::dsp::eq::{BANDS, Band, DEFAULT_BANDS};
use crate::dsp::gate::GateSettings;
use crate::shared::InputChannel;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// node.name of the capture source; none = system default
    pub input: Option<String>,
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
    pub freq: f32,
    pub gain: f32,
    pub q: f32,
}

impl From<Band> for BandConfig {
    fn from(b: Band) -> Self {
        Self {
            on: b.on,
            freq: b.freq,
            gain: b.gain,
            q: b.q,
        }
    }
}

impl From<BandConfig> for Band {
    fn from(b: BandConfig) -> Self {
        Self {
            on: b.on,
            freq: b.freq.clamp(20.0, 20_000.0),
            gain: b.gain.clamp(-24.0, 24.0),
            q: b.q.clamp(0.1, 10.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EqConfig {
    /// high-pass, low shelf, peak 1, peak 2, high shelf
    pub bands: Vec<BandConfig>,
}

impl Default for EqConfig {
    fn default() -> Self {
        Self {
            bands: DEFAULT_BANDS.iter().map(|&b| b.into()).collect(),
        }
    }
}

impl EqConfig {
    pub fn to_bands(&self) -> [Band; BANDS] {
        std::array::from_fn(|i| self.bands.get(i).map_or(DEFAULT_BANDS[i], |&b| b.into()))
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
        let mut c = Config::default();
        c.input = Some("alsa_input.usb-Yamaha_Steinberg_UR22C".into());
        c.input_channel = InputChannel::Right;
        c.chain.reverse();
        c.chain.push(ChainEntry::Clap {
            enabled: false,
            name: "Some Comp".into(),
            id: "org.example.comp".into(),
            path: "/run/current-system/sw/lib/clap/comp.clap".into(),
            state: Some("AAEC".into()),
        });
        c.eq.bands[2].gain = -3.5;
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
