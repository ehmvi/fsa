//! finding clap plugins on disk.

use std::collections::HashSet;
use std::ffi::CStr;
use std::path::{Path, PathBuf};

use clack_host::prelude::PluginEntry;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginInfo {
    /// path to the .clap file (the shared object; on macos the bundle binary)
    pub path: PathBuf,
    pub id: String,
    pub name: String,
    pub vendor: String,
}

/// clap search path, in priority order:
/// $CLAP_PATH, ~/.clap, the nixos system profile, every profile in
/// $NIX_PROFILES, the home-manager per-user profile, and the fhs locations.
pub fn search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(p) = std::env::var_os("CLAP_PATH") {
        paths.extend(std::env::split_paths(&p));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if let Some(home) = &home {
        paths.push(home.join(".clap"));
    }
    paths.push("/run/current-system/sw/lib/clap".into());
    if let Ok(profiles) = std::env::var("NIX_PROFILES") {
        // space separated, lowest priority first
        for p in profiles.split_whitespace().rev() {
            paths.push(Path::new(p).join("lib/clap"));
        }
    }
    if let Ok(user) = std::env::var("USER") {
        paths.push(PathBuf::from(format!(
            "/etc/profiles/per-user/{user}/lib/clap"
        )));
    }
    if let Some(home) = &home {
        paths.push(home.join(".nix-profile/lib/clap"));
    }
    paths.push("/usr/lib/clap".into());
    paths.push("/usr/local/lib/clap".into());

    let mut seen = HashSet::new();
    paths.retain(|p| seen.insert(p.clone()));
    paths
}

/// all `.clap` binaries under `dir`, following symlinks (nix profiles are
/// symlink farms)
fn find_binaries(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_clap = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("clap"));
        let meta = std::fs::metadata(&path); // follows symlinks
        let Ok(meta) = meta else { continue };
        if is_clap && meta.is_file() {
            out.push(path);
        } else if is_clap && meta.is_dir() {
            // macos bundle: Foo.clap/Contents/MacOS/Foo
            let stem = path.file_stem().unwrap_or_default();
            let bin = path.join("Contents/MacOS").join(stem);
            if bin.is_file() {
                out.push(bin);
            }
        } else if meta.is_dir() && depth < 4 {
            find_binaries(&path, depth + 1, out);
        }
    }
}

fn cstr(s: Option<&CStr>) -> String {
    s.map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// reads the descriptors of one plugin binary
pub fn describe(path: &Path) -> anyhow::Result<Vec<PluginInfo>> {
    // safety: loading a plugin runs its code; that is the point of a host
    let entry = unsafe { PluginEntry::load(path) }.map_err(|e| anyhow::anyhow!("{e}"))?;
    let factory = entry
        .get_plugin_factory()
        .ok_or_else(|| anyhow::anyhow!("no plugin factory"))?;
    Ok(factory
        .plugin_descriptors()
        .filter_map(|d| {
            let id = cstr(d.id());
            (!id.is_empty()).then(|| PluginInfo {
                path: path.to_path_buf(),
                id,
                name: cstr(d.name()),
                vendor: cstr(d.vendor()),
            })
        })
        .collect())
}

/// scans every search path. slow-ish (loads every binary), run off the gui thread.
pub fn scan() -> Vec<PluginInfo> {
    let mut binaries = Vec::new();
    for dir in search_paths() {
        find_binaries(&dir, 0, &mut binaries);
    }
    // the same store path is often reachable through several profiles
    let mut seen = HashSet::new();
    binaries.retain(|p| seen.insert(std::fs::canonicalize(p).unwrap_or_else(|_| p.clone())));

    let mut plugins = Vec::new();
    for path in binaries {
        match describe(&path) {
            Ok(found) => plugins.extend(found),
            Err(e) => log::warn!("skipping {}: {e:#}", path.display()),
        }
    }
    plugins.sort_by_key(|p| (p.name.to_lowercase(), p.vendor.to_lowercase()));
    plugins.dedup_by(|a, b| a.id == b.id);
    plugins
}
