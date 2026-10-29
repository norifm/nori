//! Installing a plugin: a `.noriplugin` (a zip of `plugin.wasm`, `strings/`, an icon) or a folder
//! holding the same. It is checked before anything is replaced: its manifest (in the wasm), its API
//! version, and that it imports only the host interfaces this nori links.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::manifest::{Manifest, Refusal};
use crate::runtime;

/// The manifest kept beside an installed plugin.
pub(crate) const MANIFEST: &str = "manifest.bin";

/// The files a package may hold.
fn kept(name: &str) -> bool {
    matches!(name, "plugin.wasm" | "icon.svg") || (name.starts_with("strings/") && name.ends_with(".toml") && !name[8..].contains('/'))
}

/// Its files by name.
pub(crate) fn files(from: &Path) -> Result<Vec<(String, Vec<u8>)>, Refusal> {
    let unreadable = |e: &dyn std::fmt::Display| Refusal::Unreadable(e.to_string());
    if from.is_dir() {
        let mut out = Vec::new();
        for name in ["plugin.wasm", "icon.svg"] {
            if let Ok(b) = fs::read(from.join(name)) {
                out.push((name.to_string(), b));
            }
        }
        if let Ok(entries) = fs::read_dir(from.join("strings")) {
            for e in entries.flatten() {
                let name = format!("strings/{}", e.file_name().to_string_lossy());
                if kept(&name) {
                    out.push((name, fs::read(e.path()).map_err(|e| unreadable(&e))?));
                }
            }
        }
        return Ok(out);
    }
    let mut zip = zip::ZipArchive::new(fs::File::open(from).map_err(|e| unreadable(&e))?).map_err(|e| unreadable(&e))?;
    let mut out = Vec::new();
    for k in 0..zip.len() {
        let mut f = zip.by_index(k).map_err(|e| unreadable(&e))?;
        let name = f.name().to_string();
        if f.is_file() && kept(&name) {
            let mut b = Vec::new();
            f.read_to_end(&mut b).map_err(|e| unreadable(&e))?;
            out.push((name, b));
        }
    }
    Ok(out)
}

/// Installs the plugin at `from` into its folder under `root`, replacing an older version but its store.
pub(crate) fn install(from: &Path, root: &Path) -> Result<Manifest, Refusal> {
    let files = files(from)?;
    let get = |n: &str| files.iter().find(|(name, _)| name == n).map(|(_, b)| b);
    let wasm = get("plugin.wasm").ok_or_else(|| Refusal::Unreadable("no plugin.wasm".into()))?;
    let section = Manifest::section(wasm)?;
    let manifest = Manifest::of_section(&section)?;
    if manifest.extends("sound-stage") && manifest.stage.is_none() {
        return Err(Refusal::Unreadable("a sound stage without its slot".into()));
    }
    // Written beside the old version first, and checked there.
    let staging: PathBuf = root.join(format!(".{}.new", manifest.id));
    let _ = fs::remove_dir_all(&staging);
    let unwritable = |e: std::io::Error| Refusal::Unreadable(e.to_string());
    fs::create_dir_all(staging.join("strings")).map_err(unwritable)?;
    for (name, bytes) in &files {
        fs::write(staging.join(name), bytes).map_err(unwritable)?;
    }
    // Read at every start in place of the wasm.
    fs::write(staging.join(MANIFEST), &section).map_err(unwritable)?;
    let component = runtime::component(&staging).inspect_err(|_| {
        let _ = fs::remove_dir_all(&staging);
    })?;
    let missing = runtime::missing(&component);
    if !missing.is_empty() {
        let _ = fs::remove_dir_all(&staging);
        return Err(Refusal::Needs(missing));
    }
    let folder = root.join(&manifest.id);
    if let Ok(store) = fs::metadata(folder.join("store")).map(|_| folder.join("store")) {
        let _ = fs::rename(store, staging.join("store"));
    }
    let _ = fs::remove_dir_all(&folder);
    fs::rename(&staging, &folder).map_err(unwritable)?;
    Ok(manifest)
}
