//! MSI generation through the WiX toolset.
//!
//! WiX is an external dependency (candle + light from WiX 3.x, or the wix CLI). This module writes
//! the .wxs description from the staged payload and then compiles it, so no WiX project files live
//! in the user's repository.
//!
//! The package installs per machine into Program Files, adds a Start Menu shortcut and registers
//! itself in Add/Remove Programs, which is what MSI gives for free once the tables are correct.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Everything the package needs to know about one application.
pub struct MsiSpec<'a> {
    pub app_name: &'a str,
    pub app_version: &'a str,
    /// Reverse-DNS application id; seeds the upgrade code.
    pub app_id: &'a str,
    pub publisher: &'a str,
    pub executable: &'a str,
    /// Staged directory to install (executable, assets, manifest).
    pub payload: &'a Path,
    /// Package to produce.
    pub output: &'a Path,
    /// Rust target architecture name, for example x86_64.
    pub arch: &'a str,
}

/// A file that goes into the installer.
struct PayloadFile {
    relative: String,
    source: PathBuf,
}

pub fn build(spec: &MsiSpec<'_>) -> Result<PathBuf> {
    if !spec.payload.is_dir() {
        bail!("payload directory not found: {}", spec.payload.display());
    }
    let files = collect(spec.payload)?;
    if files.is_empty() {
        bail!("payload directory is empty: {}", spec.payload.display());
    }
    let version = spec.app_version.trim();
    if version.is_empty() {
        bail!("app.version is empty");
    }
    let platform = match spec.arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "x86",
        other => bail!("unsupported MSI architecture '{other}'"),
    };

    let wxs = generate(spec, &files, version, platform)?;
    let stem = spec
        .output
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "installer".to_owned());
    let work = spec
        .payload
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let wxs_path = work.join(format!("{stem}.wxs"));
    let wixobj = work.join(format!("{stem}.wixobj"));
    std::fs::write(&wxs_path, &wxs).with_context(|| format!("writing {}", wxs_path.display()))?;

    let tools = find_wix()?;
    let candle = Command::new(&tools.candle)
        .arg("-arch")
        .arg(platform)
        .arg("-out")
        .arg(&wixobj)
        .arg(&wxs_path)
        .output()
        .with_context(|| format!("running {}", tools.candle.display()))?;
    if !candle.status.success() {
        bail!(
            "candle failed ({}): {}",
            candle.status,
            tail(&String::from_utf8_lossy(&candle.stdout), 20)
        );
    }
    let light = Command::new(&tools.light)
        .arg("-out")
        .arg(spec.output)
        .arg(&wixobj)
        .output()
        .with_context(|| format!("running {}", tools.light.display()))?;
    if !light.status.success() {
        bail!(
            "light failed ({}): {}",
            light.status,
            tail(&String::from_utf8_lossy(&light.stdout), 20)
        );
    }
    if !spec.output.is_file() {
        bail!(
            "light reported success but {} was not produced",
            spec.output.display()
        );
    }
    Ok(spec.output.to_path_buf())
}

/// Render the WiX source. Separated so tests can assert the tables without WiX installed.
fn generate(
    spec: &MsiSpec<'_>,
    files: &[PayloadFile],
    version: &str,
    platform: &str,
) -> Result<String> {
    let product_code = guid(&format!("{}:{}", spec.app_id, version));
    let upgrade_code = guid(&format!("{}:upgrade", spec.app_id));
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    s.push_str("<Wix xmlns=\"http://schemas.microsoft.com/wix/2006/wi\">\n");
    s.push_str(&format!(
        "  <Product Id=\"{product_code}\" Name=\"{}\" Language=\"1033\" Version=\"{version}\" Manufacturer=\"{}\" UpgradeCode=\"{upgrade_code}\">\n",
        xml(spec.app_name),
        xml(spec.publisher)
    ));
    s.push_str(&format!(
        "    <Package InstallerVersion=\"500\" Compressed=\"yes\" InstallScope=\"perMachine\" Platform=\"{platform}\" />\n"
    ));
    s.push_str(
        "    <MajorUpgrade DowngradeErrorMessage=\"A newer version is already installed.\" />\n",
    );
    s.push_str("    <MediaTemplate EmbedCab=\"yes\" />\n");
    s.push_str("    <Directory Id=\"TARGETDIR\" Name=\"SourceDir\">\n");
    s.push_str("      <Directory Id=\"ProgramFiles64Folder\">\n");
    s.push_str(&format!(
        "        <Directory Id=\"INSTALLFOLDER\" Name=\"{}\">\n",
        xml(spec.app_name)
    ));
    for (index, file) in files.iter().enumerate() {
        s.push_str(&format!(
            "          <Component Id=\"cmp_{index}\" Guid=\"*\">\n            <File Id=\"fil_{index}\" Source=\"{}\" KeyPath=\"yes\" />\n          </Component>\n",
            xml(&file.source.to_string_lossy())
        ));
    }
    s.push_str("        </Directory>\n");
    s.push_str("      </Directory>\n");
    s.push_str("      <Directory Id=\"ProgramMenuFolder\">\n");
    s.push_str(&format!(
        "        <Directory Id=\"AppMenuFolder\" Name=\"{}\" />\n",
        xml(spec.app_name)
    ));
    s.push_str("      </Directory>\n");
    s.push_str("    </Directory>\n");
    s.push_str("    <DirectoryRef Id=\"AppMenuFolder\">\n");
    s.push_str("      <Component Id=\"cmp_shortcut\" Guid=\"*\">\n");
    s.push_str(&format!(
        "        <Shortcut Id=\"app_shortcut\" Name=\"{}\" Target=\"[INSTALLFOLDER]{}\" WorkingDirectory=\"INSTALLFOLDER\" />\n",
        xml(spec.app_name),
        xml(spec.executable)
    ));
    s.push_str("        <RemoveFolder Id=\"rm_menu_folder\" On=\"uninstall\" />\n");
    s.push_str(&format!(
        "        <RegistryValue Root=\"HKCU\" Key=\"Software\\{}\" Name=\"installed\" Type=\"integer\" Value=\"1\" KeyPath=\"yes\" />\n",
        xml(spec.app_id)
    ));
    s.push_str("      </Component>\n");
    s.push_str("    </DirectoryRef>\n");
    s.push_str(&format!(
        "    <Feature Id=\"MainFeature\" Title=\"{}\" Level=\"1\">\n",
        xml(spec.app_name)
    ));
    for index in 0..files.len() {
        s.push_str(&format!("      <ComponentRef Id=\"cmp_{index}\" />\n"));
    }
    s.push_str("      <ComponentRef Id=\"cmp_shortcut\" />\n");
    s.push_str("    </Feature>\n");
    s.push_str("  </Product>\n");
    s.push_str("</Wix>\n");
    Ok(s)
}

fn collect(payload: &Path) -> Result<Vec<PayloadFile>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<PayloadFile>) -> Result<()> {
        for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                walk(root, &path, out)?;
            } else if file_type.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push(PayloadFile {
                    relative,
                    source: path,
                });
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(payload, payload, &mut files)?;
    files.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(files)
}

/// Deterministic GUID derived from a seed, so upgrades keep a stable UpgradeCode.
fn guid(seed: &str) -> String {
    let digest = Sha256::digest(seed.as_bytes());
    let bytes = &digest[..16];
    let mut value = [0u8; 16];
    value.copy_from_slice(bytes);
    value[6] = (value[6] & 0x0f) | 0x40;
    value[8] = (value[8] & 0x3f) | 0x80;
    format!(
        "{{{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        value[0],
        value[1],
        value[2],
        value[3],
        value[4],
        value[5],
        value[6],
        value[7],
        value[8],
        value[9],
        value[10],
        value[11],
        value[12],
        value[13],
        value[14],
        value[15]
    )
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn tail(text: &str, lines: usize) -> String {
    let collected: Vec<&str> = text.lines().collect();
    let start = collected.len().saturating_sub(lines);
    collected[start..].join("\n")
}

struct WixTools {
    candle: PathBuf,
    light: PathBuf,
}

/// Locate candle.exe and light.exe: WIX environment variable, PATH, then the default install.
fn find_wix() -> Result<WixTools> {
    let names = if cfg!(windows) {
        ("candle.exe", "light.exe")
    } else {
        ("candle", "light")
    };
    let mut directories: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("WIX") {
        let base = PathBuf::from(dir);
        directories.push(base.join("bin"));
        directories.push(base);
    }
    if let Some(path) = std::env::var_os("PATH") {
        directories.extend(std::env::split_paths(&path).map(|dir| dir.to_path_buf()));
    }
    if cfg!(windows) {
        if let Some(dir) = std::env::var_os("ProgramFiles(x86)") {
            let base = PathBuf::from(dir);
            for entry in ["WiX Toolset v3.14/bin", "WiX Toolset v3.11/bin"] {
                directories.push(base.join(entry));
            }
        }
    }
    for directory in directories {
        let candle = directory.join(names.0);
        let light = directory.join(names.1);
        if candle.is_file() && light.is_file() {
            return Ok(WixTools { candle, light });
        }
    }
    bail!(
        "candle/light not found; install the WiX toolset (choco install wixtoolset) or set WIXDIR"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("msi-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let payload = root.join("payload");
        std::fs::create_dir_all(payload.join("assets")).unwrap();
        std::fs::write(payload.join("app.exe"), b"exe").unwrap();
        std::fs::write(payload.join("assets/logo.png"), b"png").unwrap();
        root
    }

    #[test]
    fn generates_components_and_a_feature() {
        let root = fixture("gen");
        let payload = root.join("payload");
        let files = collect(&payload).unwrap();
        let spec = MsiSpec {
            app_name: "Winia Blank",
            app_version: "0.1.3",
            app_id: "com.example.winia_blank",
            publisher: "Winia contributors",
            executable: "app.exe",
            payload: &payload,
            output: &root.join("app.msi"),
            arch: "x86_64",
        };
        let wxs = generate(&spec, &files, "0.1.3", "x64").unwrap();
        assert!(wxs.contains("Platform=\"x64\""));
        assert!(wxs.contains("<Component Id=\"cmp_0\" Guid=\"*\">"));
        assert!(wxs.contains("<Component Id=\"cmp_1\" Guid=\"*\">"));
        assert!(wxs.contains("<ComponentRef Id=\"cmp_shortcut\" />"));
        assert!(wxs.contains("ProgramFiles64Folder"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn guids_are_deterministic_and_versioned() {
        let first = guid("com.example.app:0.1.0");
        assert_eq!(first, guid("com.example.app:0.1.0"));
        assert_ne!(first, guid("com.example.app:0.2.0"));
        assert!(first.starts_with('{') && first.ends_with('}'));
        assert_eq!(first.len(), 38);
    }

    #[test]
    fn escapes_xml_metacharacters() {
        assert_eq!(xml("a & b <c>"), "a &amp; b &lt;c&gt;");
    }
}
