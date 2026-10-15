//! lv2 hosting.
//!
//! lv2 control inputs are owned by the host, so the gui keeps the
//! authoritative values and mirrors changes to the audio thread through an
//! rtrb ring. only control values are persisted; plugins with internal
//! state beyond their controls (lv2 state extension) start fresh. plugin
//! uis are not hosted, every plugin gets generic controls.

use std::path::{Path, PathBuf};

// lv2-sys only builds on linux, and lilv 0.2 only where c_char is signed
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod host;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use host::{Lv2Host, Lv2Plugin};

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
mod stub;
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub use stub::{Lv2Host, Lv2Plugin};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lv2Info {
    pub uri: String,
    pub name: String,
    pub author: String,
}

#[derive(Clone, Debug)]
#[cfg_attr(
    not(all(target_os = "linux", target_arch = "x86_64")),
    allow(dead_code)
)]
pub struct Lv2Param {
    /// lv2 port index
    port: usize,
    pub symbol: String,
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub toggled: bool,
    pub integer: bool,
    pub logarithmic: bool,
    /// enumeration / scale points: (value, label)
    pub points: Vec<(f32, String)>,
    pub value: f32,
}

/// lv2 search path, mirroring the clap one: an existing $LV2_PATH, ~/.lv2,
/// the nixos system profile, every profile in $NIX_PROFILES, the
/// home-manager profile and the fhs locations.
pub fn search_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("LV2_PATH") {
        paths.extend(std::env::split_paths(&p));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if let Some(home) = &home {
        paths.push(home.join(".lv2"));
    }
    paths.push("/run/current-system/sw/lib/lv2".into());
    if let Ok(profiles) = std::env::var("NIX_PROFILES") {
        for p in profiles.split_whitespace().rev() {
            paths.push(Path::new(p).join("lib/lv2"));
        }
    }
    if let Ok(user) = std::env::var("USER") {
        paths.push(PathBuf::from(format!(
            "/etc/profiles/per-user/{user}/lib/lv2"
        )));
    }
    if let Some(home) = &home {
        paths.push(home.join(".nix-profile/lib/lv2"));
    }
    paths.push("/usr/lib/lv2".into());
    paths.push("/usr/local/lib/lv2".into());
    let mut seen = std::collections::HashSet::new();
    paths.retain(|p| seen.insert(p.clone()));
    paths
}

/// points lilv at our search path. lilv only reads $LV2_PATH, so this has
/// to run before any other thread exists.
///
/// # safety
/// must be called while the process is still single-threaded.
pub unsafe fn export_search_path() {
    let joined = std::env::join_paths(search_paths().into_iter().filter(|p| p.is_dir()));
    if let Ok(joined) = joined {
        // safety: guaranteed single-threaded by the caller
        unsafe { std::env::set_var("LV2_PATH", joined) };
    }
}
