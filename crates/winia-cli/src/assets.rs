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
    #[serde(default)]
    pub window_icon: Option<String>,
    pub files: Vec<AssetEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetEntry {
    /// Stable logical asset id without a density marker.
    pub id: String,
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
    let mut variants: HashMap<String, String> = HashMap::new();
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
        let (id, density) = asset_identity(&relative);
        let variant_key = format!("{}@{:.3}", id, density.unwrap_or(1.0));
        if let Some(previous) = variants.insert(variant_key, relative.clone()) {
            bail!("asset density conflict: {previous} and {relative}");
        }
        entries.push(AssetEntry {
            id,
            packaged: relative.clone(),
            path: relative.clone(),
            bytes: bytes.len() as u64,
            sha256: format!("{digest:x}"),
            density,
        });
    }

    Ok(AssetManifest {
        version: 2,
        window_icon: None,
        files: entries,
    })
}

pub fn stage_desktop(root: &Path, config: &WiniaConfig, stage: &Path) -> Result<AssetManifest> {
    let mut manifest = collect(root, config)?;
    for entry in &mut manifest.files {
        let destination = stage.join(&entry.packaged);
        copy_entry(root, entry, &destination)?;
    }
    if let Some(icons) = crate::icons::ensure(root, config)? {
        let relative = "assets/winia/window-icon.png";
        let destination = stage.join(relative);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&icons.store_png, &destination)
            .with_context(|| format!("copying {}", icons.store_png.display()))?;
        let bytes = std::fs::read(&destination)?;
        let digest = Sha256::digest(&bytes);
        manifest.files.push(AssetEntry {
            id: relative.to_owned(),
            path: relative.to_owned(),
            packaged: relative.to_owned(),
            bytes: bytes.len() as u64,
            sha256: format!("{digest:x}"),
            density: None,
        });
        manifest.window_icon = Some(relative.to_owned());
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
    if let Some(icons) = crate::icons::ensure(root, config)? {
        let relative = "winia/window-icon.png";
        let target = destination.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&icons.store_png, &target)?;
        let bytes = std::fs::read(&target)?;
        let digest = Sha256::digest(&bytes);
        let logical = format!("assets/{relative}");
        manifest.files.push(AssetEntry {
            id: logical.clone(),
            path: logical.clone(),
            packaged: format!("winia-assets/{relative}"),
            bytes: bytes.len() as u64,
            sha256: format!("{digest:x}"),
            density: None,
        });
        manifest.window_icon = Some(logical);
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

fn asset_identity(path: &str) -> (String, Option<f32>) {
    let mut segments: Vec<String> = path.split('/').map(str::to_owned).collect();
    let mut density = None;

    if let Some(index) = segments
        .iter()
        .take(segments.len().saturating_sub(1))
        .position(|segment| density_token(segment).is_some())
    {
        density = density_token(&segments[index]);
        segments.remove(index);
    }

    if let Some(filename) = segments.pop() {
        let (stem, extension) = filename
            .rsplit_once('.')
            .map_or((filename.as_str(), None), |(stem, extension)| {
                (stem, Some(extension))
            });
        let (stem, suffix_density) = match stem.rsplit_once('@') {
            Some((stem, token)) => match density_token(token) {
                Some(value) => (stem, Some(value)),
                None => (stem, None),
            },
            None => (stem, None),
        };
        if suffix_density.is_some() {
            density = suffix_density;
        }
        segments.push(match extension {
            Some(extension) => format!("{stem}.{extension}"),
            None => stem.to_owned(),
        });
    }

    (segments.join("/"), density)
}

fn density_token(value: &str) -> Option<f32> {
    let value = value.strip_suffix('x')?;
    value.parse::<f32>().ok().filter(|value| *value > 0.0)
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
    fn identifies_density_variants() {
        assert_eq!(
            asset_identity("assets/icons/logo@2x.png"),
            ("assets/icons/logo.png".to_owned(), Some(2.0))
        );
        assert_eq!(
            asset_identity("assets/icons/2x/logo.png"),
            ("assets/icons/logo.png".to_owned(), Some(2.0))
        );
        assert_eq!(
            asset_identity("assets/icons/logo@1.5x.svg"),
            ("assets/icons/logo.svg".to_owned(), Some(1.5))
        );
        assert_eq!(
            asset_identity("assets/icons/logo.png"),
            ("assets/icons/logo.png".to_owned(), None)
        );
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
