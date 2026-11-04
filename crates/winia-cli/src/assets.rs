//! Project asset collection, manifest generation, and validation.
//!
//! Assets are declared in `winia.toml` and copied into the platform staging
//! directory at build/package time. The manifest records the logical path,
//! packaged path, size, SHA-256, and optional density variant.

use crate::config::WiniaConfig;
use anyhow::{Context, Result, bail};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetManifest {
    pub version: u32,
    pub files: Vec<AssetEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetEntry {
    /// Project-relative logical path, using forward slashes.
    pub path: String,
    /// Path inside the platform bundle.
    pub packaged: String,
    pub bytes: u64,
    pub sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub density: Option<f32>,
}

pub fn collect(root: &Path, config: &WiniaConfig) -> Result<AssetManifest> {
    let include = build_set(&config.assets.include)?;
    let exclude = build_set(&config.assets.exclude)?;
    let mut files = Vec::new();
    walk(root, root, &include, &exclude, &mut files)?;
    files.sort();

    let mut entries = Vec::new();
    let mut seen: HashMap<String, String> = HashMap::new();
    for path in files {
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if relative.is_empty() || relative.starts_with("../") || relative.contains("/../") {
            bail!("asset path escapes the project root: {relative}");
        }
        let lower = relative.to_lowercase();
        if let Some(previous) = seen.insert(lower, relative.clone()) {
            bail!("asset path conflicts by case: {previous} and {relative}");
        }
        let bytes =
            std::fs::read(&path).with_context(|| format!("reading asset {}", path.display()))?;
        let digest = Sha256::digest(&bytes);
        entries.push(AssetEntry {
            packaged: relative.clone(),
            path: relative.clone(),
            bytes: bytes.len() as u64,
            sha256: format!("{digest:x}"),
            density: density_of(&relative),
        });
    }

    Ok(AssetManifest {
        version: 1,
        files: entries,
    })
}

pub fn stage_desktop(root: &Path, config: &WiniaConfig, stage: &Path) -> Result<AssetManifest> {
    let mut manifest = collect(root, config)?;
    for entry in &mut manifest.files {
        let destination = stage.join(&entry.packaged);
        copy_entry(root, entry, &destination)?;
    }
    write_manifest(&stage.join("assets.manifest.json"), &manifest)?;
    Ok(manifest)
}

pub fn stage_android(root: &Path, config: &WiniaConfig) -> Result<AssetManifest> {
    let android = root.join("android");
    if !android.is_dir() {
        bail!("android/ not found; cannot stage assets");
    }
    let destination = android.join("app/src/main/assets/winia-assets");
    if destination.exists() {
        std::fs::remove_dir_all(&destination)
            .with_context(|| format!("removing {}", destination.display()))?;
    }
    std::fs::create_dir_all(&destination)
        .with_context(|| format!("creating {}", destination.display()))?;

    let mut manifest = collect(root, config)?;
    for entry in &mut manifest.files {
        let relative = entry
            .path
            .strip_prefix("assets/")
            .unwrap_or(&entry.path)
            .to_owned();
        entry.packaged = format!("winia-assets/{relative}");
        let target = destination.join(&relative);
        copy_entry(root, entry, &target)?;
    }
    write_manifest(&destination.join("manifest.json"), &manifest)?;
    Ok(manifest)
}

fn copy_entry(root: &Path, entry: &AssetEntry, destination: &Path) -> Result<()> {
    let source = root.join(&entry.path);
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::copy(&source, destination)
        .with_context(|| format!("copying {} to {}", source.display(), destination.display()))?;
    let data =
        std::fs::read(destination).with_context(|| format!("reading {}", destination.display()))?;
    let digest = Sha256::digest(&data);
    if data.len() as u64 != entry.bytes || format!("{digest:x}") != entry.sha256 {
        bail!("asset verification failed for {}", entry.path);
    }
    Ok(())
}

fn write_manifest(path: &Path, manifest: &AssetManifest) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, serde_json::to_string_pretty(manifest)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn build_set(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .with_context(|| format!("invalid asset glob '{pattern}'"))?;
        builder.add(glob);
    }
    builder.build().context("building asset glob set")
}

fn walk(
    root: &Path,
    directory: &Path,
    include: &GlobSet,
    exclude: &GlobSet,
    files: &mut Vec<PathBuf>,
) -> Result<()> {
    for entry in
        std::fs::read_dir(directory).with_context(|| format!("reading {}", directory.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if file_type.is_dir() {
            if skip_directory(&relative) {
                continue;
            }
            walk(root, &path, include, exclude, files)?;
        } else if file_type.is_file() && include.is_match(&relative) && !exclude.is_match(&relative)
        {
            files.push(path);
        }
    }
    Ok(())
}

fn skip_directory(relative: &str) -> bool {
    matches!(
        relative,
        "target" | "dist" | ".git" | "android/.gradle" | "android/app/build" | "android/build"
    ) || relative.starts_with("target/")
        || relative.starts_with("android/app/src/main/assets/winia-assets")
}

fn density_of(path: &str) -> Option<f32> {
    path.split('/').find_map(|segment| {
        segment
            .strip_suffix('x')
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|value| *value > 0.0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("winia-assets-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("assets/icons")).unwrap();
        root
    }

    #[test]
    fn collects_and_hashes_assets() {
        let root = temp_root("collect");
        std::fs::write(root.join("assets/icons/logo.svg"), "<svg/>").unwrap();
        std::fs::write(root.join("assets/notes.psd"), "psd").unwrap();
        let config = WiniaConfig::default();
        let manifest = collect(&root, &config).unwrap();
        assert_eq!(manifest.files.len(), 1);
        assert_eq!(manifest.files[0].path, "assets/icons/logo.svg");
        assert_eq!(manifest.files[0].bytes, 6);
        assert_eq!(manifest.files[0].sha256.len(), 64);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn stages_desktop_manifest() {
        let root = temp_root("desktop");
        std::fs::write(root.join("assets/icons/logo.svg"), "<svg/>").unwrap();
        let stage = root.join("stage");
        let config = WiniaConfig::default();
        stage_desktop(&root, &config, &stage).unwrap();
        assert!(stage.join("assets/icons/logo.svg").is_file());
        assert!(stage.join("assets.manifest.json").is_file());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
