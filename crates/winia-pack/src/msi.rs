//! MSI generation through the WiX toolset.
//!
//! WiX is an external dependency (candle + light from WiX 3.x). This module writes the .wxs
//! description from the staged payload and then compiles it, so no WiX project files live in the
//! user's repository.
//!
//! The package installs per machine into Program Files, mirrors the payload directory tree, adds a
//! Start Menu shortcut and registers itself in Add/Remove Programs.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
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
    /// Path relative to the payload root, forward slashes.
    relative: String,
    source: PathBuf,
}

/// One directory in the installer tree.
#[derive(Default)]
struct DirNode {
    children: BTreeMap<String, DirNode>,
    /// File name and source path, sorted by name.
    files: Vec<(String, PathBuf)>,
}

impl DirNode {
    fn insert(&mut self, relative: &str, source: &Path) {
        let mut parts = relative.split('/').peekable();
        let mut node = self;
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                node.files.push((part.to_owned(), source.to_path_buf()));
                return;
            }
            node = node.children.entry(part.to_owned()).or_default();
        }
    }
}

/// Four numeric fields; pre-release and build metadata are dropped.
fn numeric_version(version: &str) -> String {
    let core = version.split(['-', '+']).next().unwrap_or("0.0.0");
    let mut parts = core
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0))
        .collect::<Vec<_>>();
    parts.resize(4, 0);
    parts.truncate(4);
    parts
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

pub fn build(spec: &MsiSpec<'_>) -> Result<PathBuf> {
    if !spec.payload.is_dir() {
        bail!("payload directory not found: {}", spec.payload.display());
    }
    crate::validate_app_name(spec.app_name)?;
    if let Some(parent) = spec.output.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let files = collect(spec.payload)?;
    if files.is_empty() {
        bail!("payload directory is empty: {}", spec.payload.display());
    }
    let version = spec.app_version.trim();
    if version.is_empty() {
        bail!("app.version is empty");
    }
    // WiX only accepts numeric fields; a semver pre-release must not reach Product/@Version.
    let product_version = numeric_version(version);
    let platform = match spec.arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "x86",
        other => bail!("unsupported MSI architecture '{other}'"),
    };

    let wxs = generate(spec, &files, &product_version, platform)?;
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
            tail(&combined(&candle.stdout, &candle.stderr), 20)
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
            tail(&combined(&light.stdout, &light.stderr), 20)
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
    // WiX rejects an empty Manufacturer, and many projects do not set one.
    let publisher = if spec.publisher.trim().is_empty() {
        spec.app_name
    } else {
        spec.publisher
    };

    let mut root = DirNode::default();
    for file in files {
        root.insert(&file.relative, &file.source);
    }

    let mut body = String::new();
    let mut components: Vec<String> = Vec::new();
    let mut counter = 0usize;
    emit_directory(
        &mut body,
        &root,
        "INSTALLFOLDER",
        &mut components,
        &mut counter,
    );

    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    s.push_str("<Wix xmlns=\"http://schemas.microsoft.com/wix/2006/wi\">\n");
    s.push_str(&format!(
        "  <Product Id=\"{product_code}\" Name=\"{}\" Language=\"1033\" Codepage=\"65001\" Version=\"{version}\" Manufacturer=\"{}\" UpgradeCode=\"{upgrade_code}\">\n",
        xml(spec.app_name),
        xml(publisher)
    ));
    s.push_str(
        "    <Package InstallerVersion=\"500\" Compressed=\"yes\" InstallScope=\"perMachine\" Platform=\"",
    );
    s.push_str(platform);
    s.push_str("\" />\n");
    s.push_str(
        "    <MajorUpgrade DowngradeErrorMessage=\"A newer version is already installed.\" />\n",
    );
    s.push_str("    <MediaTemplate EmbedCab=\"yes\" />\n");
    // The Windows payload carries app.ico; show it in Settings/Add-Remove Programs.
    let icon = spec.payload.join("app.ico");
    if icon.is_file() {
        let icon = icon.display().to_string().replace('\\', "/");
        s.push_str(&format!(
            "    <Icon Id=\"AppIcon\" SourceFile=\"{}\" />\n",
            xml(&icon)
        ));
        s.push_str("    <Property Id=\"ARPPRODUCTICON\" Value=\"AppIcon\" />\n");
    }
    s.push_str("    <Directory Id=\"TARGETDIR\" Name=\"SourceDir\">\n");
    // A 32-bit package cannot install into the 64-bit Program Files folder.
    let program_files = if platform == "x86" {
        "ProgramFilesFolder"
    } else {
        "ProgramFiles64Folder"
    };
    s.push_str(&format!("      <Directory Id=\"{program_files}\">\n"));
    s.push_str(&format!(
        "        <Directory Id=\"INSTALLFOLDER\" Name=\"{}\">\n",
        xml(spec.app_name)
    ));
    s.push_str(&body);
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
    for component in &components {
        s.push_str(&format!("      <ComponentRef Id=\"{component}\" />\n"));
    }
    s.push_str("      <ComponentRef Id=\"cmp_shortcut\" />\n");
    s.push_str("    </Feature>\n");
    s.push_str("  </Product>\n");
    s.push_str("</Wix>\n");
    Ok(s)
}

/// Emit nested Directory elements so the payload keeps its shape after installation.
fn emit_directory(
    out: &mut String,
    node: &DirNode,
    id: &str,
    components: &mut Vec<String>,
    counter: &mut usize,
) {
    for (index, (name, child)) in node.children.iter().enumerate() {
        let child_id = format!("{id}_d{index}");
        out.push_str(&format!(
            "          <Directory Id=\"{child_id}\" Name=\"{}\">\n",
            xml(name)
        ));
        emit_directory(out, child, &child_id, components, counter);
        out.push_str("          </Directory>\n");
    }
    for (name, source) in &node.files {
        let component = format!("cmp_{}", *counter);
        *counter += 1;
        out.push_str(&format!(
            "          <Component Id=\"{component}\" Guid=\"*\">\n            <File Id=\"fil_{component}\" Source=\"{}\" Name=\"{}\" KeyPath=\"yes\" />\n          </Component>\n",
            xml(&source.to_string_lossy()),
            xml(name)
        ));
        components.push(component);
    }
}

fn collect(payload: &Path) -> Result<Vec<PayloadFile>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<PayloadFile>) -> Result<()> {
        for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                eprintln!("warning: MSI payload skips symlink {}", path.display());
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
    let mut value = [0u8; 16];
    value.copy_from_slice(&digest[..16]);
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

fn combined(stdout: &[u8], stderr: &[u8]) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(stdout),
        String::from_utf8_lossy(stderr)
    )
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

/// Locate candle and light: WIX environment variable, PATH, then the default install.
pub fn wix_available() -> bool {
    find_wix().is_ok()
}

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
        directories.extend(std::env::split_paths(&path));
    }
    if cfg!(windows)
        && let Some(dir) = std::env::var_os("ProgramFiles(x86)")
    {
        let base = PathBuf::from(dir);
        for entry in ["WiX Toolset v3.14/bin", "WiX Toolset v3.11/bin"] {
            directories.push(base.join(entry));
        }
    }
    for directory in directories {
        let candle = directory.join(names.0);
        let light = directory.join(names.1);
        if candle.is_file() && light.is_file() {
            return Ok(WixTools { candle, light });
        }
    }
    bail!("candle/light not found; install the WiX toolset (choco install wixtoolset) or set WIX")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_32_bit_package_uses_the_32_bit_program_files_folder() {
        let root = fixture("x86");
        let payload = root.join("payload");
        let output = root.join("out.msi");
        let mut spec = spec(&payload, &output, "Winia");
        spec.arch = "x86";
        let files = collect(&payload).unwrap();
        let wxs = generate(&spec, &files, "0.1.3.0", "x86").unwrap();
        assert!(
            wxs.contains("<Directory Id=\"ProgramFilesFolder\">"),
            "{wxs}"
        );
        assert!(!wxs.contains("ProgramFiles64Folder"), "{wxs}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn product_versions_are_numeric() {
        assert_eq!(numeric_version("1.2.3"), "1.2.3.0");
        assert_eq!(numeric_version("1.0.0-beta.1"), "1.0.0.0");
        assert_eq!(numeric_version("2.1"), "2.1.0.0");
    }

    fn fixture(name: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("msi-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let payload = root.join("payload");
        std::fs::create_dir_all(payload.join("assets/winia")).unwrap();
        std::fs::write(payload.join("app.exe"), b"exe").unwrap();
        std::fs::write(payload.join("app.ico"), b"ico").unwrap();
        std::fs::write(payload.join("assets/winia/icon.png"), b"png").unwrap();
        root
    }

    fn spec<'a>(payload: &'a Path, output: &'a Path, publisher: &'a str) -> MsiSpec<'a> {
        MsiSpec {
            app_name: "Winia Blank",
            app_version: "0.1.3",
            app_id: "com.example.winia_blank",
            publisher,
            executable: "app.exe",
            payload,
            output,
            arch: "x86_64",
        }
    }

    #[test]
    fn mirrors_the_payload_directory_tree() {
        let root = fixture("tree");
        let payload = root.join("payload");
        let files = collect(&payload).unwrap();
        let wxs = generate(
            &spec(&payload, &root.join("app.msi"), "Winia"),
            &files,
            "0.1.3",
            "x64",
        )
        .unwrap();
        assert!(wxs.contains("Platform=\"x64\""));
        assert!(wxs.contains("<Directory Id=\"INSTALLFOLDER_d0\" Name=\"assets\">"));
        assert!(wxs.contains("<Directory Id=\"INSTALLFOLDER_d0_d0\" Name=\"winia\">"));
        assert!(wxs.contains("Name=\"icon.png\""));
        assert!(wxs.contains("<ComponentRef Id=\"cmp_shortcut\" />"));
        assert!(wxs.contains("ProgramFiles64Folder"));
        assert!(
            wxs.contains("Codepage=\"65001\""),
            "MSI database must be Unicode: {wxs}"
        );
        assert!(wxs.contains("ARPPRODUCTICON"), "{wxs}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn falls_back_to_the_app_name_when_no_publisher_is_set() {
        let root = fixture("publisher");
        let payload = root.join("payload");
        let files = collect(&payload).unwrap();
        let wxs = generate(
            &spec(&payload, &root.join("app.msi"), ""),
            &files,
            "0.1.3",
            "x64",
        )
        .unwrap();
        assert!(wxs.contains("Manufacturer=\"Winia Blank\""));
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
