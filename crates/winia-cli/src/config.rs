//! The `winia.toml` project manifest.
//!
//! It is the single source of truth for application identity, window defaults,
//! build options, and platform packaging metadata. The CLI reads it for every
//! command; future platform tools (`winia-build`, `winia-pack`) will consume
//! the same model.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WiniaConfig {
    pub app: AppConfig,
    pub window: WindowConfig,
    pub build: BuildConfig,
    pub package: PackageConfig,
    pub android: AndroidConfig,
    pub assets: AssetsConfig,
    pub hooks: HooksConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Display name shown in the window title and installers.
    pub name: String,
    /// Reverse-DNS application id, also the Android package name.
    pub id: String,
    pub version: String,
    pub authors: Vec<String>,
    pub license: Option<String>,
    /// Source image used to generate platform icon sets.
    pub icon: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            name: "My App".to_owned(),
            id: "com.example.myapp".to_owned(),
            version: "0.1.0".to_owned(),
            authors: Vec::new(),
            license: None,
            icon: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    pub title: String,
    pub width: f32,
    pub height: f32,
    pub resizable: bool,
    pub decorations: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "My App".to_owned(),
            width: 1200.0,
            height: 800.0,
            resizable: true,
            decorations: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BuildConfig {
    /// Cargo features enabled for application builds.
    pub features: Vec<String>,
    /// Render backend preference: auto, vulkan, gl, cpu.
    pub backend: String,
    /// Build profile: dev, release, release-lto, minsize.
    pub profile: String,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            features: Vec::new(),
            backend: "auto".to_owned(),
            profile: "dev".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PackageConfig {
    pub windows: PlatformPackage,
    pub macos: PlatformPackage,
    pub linux: PlatformPackage,
    pub android: PlatformPackage,
}

impl Default for PackageConfig {
    fn default() -> Self {
        Self {
            windows: PlatformPackage::new(&["zip", "nsis", "msi"]),
            macos: PlatformPackage::new(&["app", "dmg"]),
            linux: PlatformPackage::new(&["tar.gz", "deb", "appimage"]),
            android: PlatformPackage::new(&["apk", "aab"]),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PlatformPackage {
    pub formats: Vec<String>,
}

impl PlatformPackage {
    fn new(formats: &[&str]) -> Self {
        Self {
            formats: formats.iter().map(|s| (*s).to_owned()).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AndroidConfig {
    /// Android application id (the same value as app.id unless overridden).
    pub package: String,
    /// Launcher label. Falls back to app.name when absent.
    pub label: Option<String>,
    /// Minimum API level. Skia bindings build against API 26.
    pub min_sdk: u32,
    pub target_sdk: u32,
    pub compile_sdk: u32,
    /// Activity base class: game (GameActivity) or native (NativeActivity).
    pub activity: AndroidActivity,
    pub version_code: u32,
    /// Android ABI names, e.g. arm64-v8a, x86_64.
    pub abis: Vec<String>,
}

impl Default for AndroidConfig {
    fn default() -> Self {
        Self {
            package: "com.example.myapp".to_owned(),
            label: None,
            min_sdk: 26,
            target_sdk: 36,
            compile_sdk: 36,
            activity: AndroidActivity::Game,
            version_code: 1,
            abis: vec!["arm64-v8a".to_owned(), "x86_64".to_owned()],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AndroidActivity {
    #[default]
    Game,
    Native,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AssetsConfig {
    /// Project-relative globs included in the app bundle.
    pub include: Vec<String>,
    /// Project-relative globs excluded from the app bundle.
    pub exclude: Vec<String>,
}

impl Default for AssetsConfig {
    fn default() -> Self {
        Self {
            include: vec!["assets/**".to_owned()],
            exclude: vec!["**/*.psd".to_owned()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct HooksConfig {
    pub before_build: Vec<String>,
    pub after_build: Vec<String>,
    pub before_package: Vec<String>,
    pub after_package: Vec<String>,
}

impl WiniaConfig {
    /// Parse a manifest from text.
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        toml::from_str(text).context("parsing winia.toml")
    }

    /// Read a manifest from disk.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text)
    }

    /// Write this manifest as pretty TOML.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let text = toml::to_string_pretty(self).context("serialising winia.toml")?;
        std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
    }

    /// Basic cross-field validation used by `winia doctor`.
    pub fn validate(&self) -> anyhow::Result<()> {
        if !is_reverse_dns(&self.app.id) {
            anyhow::bail!(
                "app.id must be reverse-DNS such as com.example.app, got '{}'",
                self.app.id
            );
        }
        if !self.android.package.trim().is_empty() && !is_java_package(&self.android.package) {
            anyhow::bail!(
                "android.package must be a Java package name (letters, digits, underscores), got '{}'",
                self.android.package
            );
        }
        if semver::Version::parse(&self.app.version).is_err() {
            anyhow::bail!(
                "app.version must be semver such as 0.1.0, got '{}'",
                self.app.version
            );
        }
        if self.android.package.trim().is_empty() {
            anyhow::bail!("android.package must not be empty");
        }
        if self.android.min_sdk < 26 {
            anyhow::bail!(
                "android.min_sdk must be at least 26 (Skia's Android build targets API 26), got {}",
                self.android.min_sdk
            );
        }
        for abi in &self.android.abis {
            crate::platform::android::Abi::parse(abi)?;
        }
        Ok(())
    }
}

/// Reverse-DNS: at least two segments, each starting with a letter and limited to letters, digits
/// and underscores. Deliberately strict about the first character so an id cannot become a label
/// fragment when it reaches the Android manifest.
fn is_reverse_dns(value: &str) -> bool {
    let segments: Vec<&str> = value.split('.').collect();
    if segments.len() < 2 {
        return false;
    }
    segments.iter().all(|segment| {
        let mut chars = segment.chars();
        matches!(chars.next(), Some(first) if first.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    })
}

/// Java package segments: letters/digits/underscores only. Hyphens are legal in a bundle id but not
/// in an Android applicationId, so the two are validated separately.
fn is_java_package(value: &str) -> bool {
    let segments: Vec<&str> = value.split('.').collect();
    if segments.len() < 2 {
        return false;
    }
    segments.iter().all(|segment| {
        let mut chars = segment.chars();
        matches!(chars.next(), Some(first) if first.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Search upwards from `start` for a `winia.toml`.
pub fn find_config(start: &Path) -> Option<PathBuf> {
    let mut dir = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };
    loop {
        let candidate = dir.join("winia.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_ids_that_are_not_reverse_dns() {
        let mut config = WiniaConfig::default();
        for id in ["myapp", "com..app", "com.1app"] {
            config.app.id = id.to_owned();
            assert!(config.validate().is_err(), "{id} should be rejected");
        }
        // Hyphens are fine in a bundle id; the Android package is checked separately.
        for id in ["com.example.my_app", "com.example.ci-smoke"] {
            config.app.id = id.to_owned();
            assert!(config.validate().is_ok(), "{id} should be accepted");
        }
    }

    #[test]
    fn rejects_android_packages_that_are_not_java_identifiers() {
        let mut config = WiniaConfig::default();
        config.app.id = "com.example.app".to_owned();
        for package in ["com.example.ci-smoke", "myapp", "com.1example"] {
            config.android.package = package.to_owned();
            assert!(
                config.validate().is_err(),
                "{package} should be rejected as an applicationId"
            );
        }
        config.android.package = "com.example.ci_smoke".to_owned();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn rejects_versions_that_are_not_semver() {
        let mut config = WiniaConfig::default();
        config.app.id = "com.example.app".to_owned();
        config.app.version = "0.1".to_owned();
        assert!(config.validate().is_err());
        config.app.version = "0.1.0-beta.1".to_owned();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn defaults_match_the_documented_android_baseline() {
        let config = WiniaConfig::default();
        assert_eq!(config.android.min_sdk, 26);
        assert_eq!(config.android.activity, AndroidActivity::Game);
        assert_eq!(config.android.abis, vec!["arm64-v8a", "x86_64"]);
        assert_eq!(config.build.backend, "auto");
        assert_eq!(config.build.profile, "dev");
        assert!(config.validate().is_ok());
    }

    #[test]
    fn parses_a_partial_manifest_and_fills_defaults() {
        let config = WiniaConfig::parse(
            r#"
[app]
name = "Demo"
id = "com.example.demo"

[android]
package = "com.example.demo"
activity = "native"
"#,
        )
        .expect("manifest should parse");

        assert_eq!(config.app.name, "Demo");
        assert_eq!(config.app.version, "0.1.0");
        assert_eq!(config.android.package, "com.example.demo");
        assert_eq!(config.android.activity, AndroidActivity::Native);
        assert_eq!(config.android.min_sdk, 26);
        assert_eq!(config.package.android.formats, vec!["apk", "aab"]);
    }

    #[test]
    fn rejects_an_android_sdk_below_26() {
        let mut config = WiniaConfig::default();
        config.android.min_sdk = 21;
        let error = config.validate().expect_err("min_sdk 21 must be rejected");
        assert!(error.to_string().contains("at least 26"));
    }
}
