//! Debian package generation for Linux desktop builds.
//!
//! A .deb is an ar archive with three members: debian-binary, control.tar.gz and data.tar.gz.
//! The payload is installed under /usr/lib/<app>, with a symlink in /usr/bin and a desktop entry,
//! which is the layout Debian policy expects for an application that ships its own data files.

use anyhow::{Context, Result, bail};
use flate2::Compression;
use flate2::write::GzEncoder;
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Everything the package needs to know about one application.
pub struct DebSpec<'a> {
    pub app_name: &'a str,
    pub app_version: &'a str,
    /// Reverse-DNS application id; becomes the Debian package name.
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

/// A single file to place inside the package.
struct PayloadFile {
    /// Path relative to the install directory, forward slashes.
    relative: String,
    bytes: Vec<u8>,
    executable: bool,
}

pub fn build(spec: &DebSpec<'_>) -> Result<PathBuf> {
    if !spec.payload.is_dir() {
        bail!("payload directory not found: {}", spec.payload.display());
    }
    let package = debian_name(spec.app_id);
    let version = debian_version(spec.app_version)?;
    let arch = debian_arch(spec.arch)?;
    let files = collect(spec.payload, spec.executable)?;
    if files.is_empty() {
        bail!("payload directory is empty: {}", spec.payload.display());
    }
    let stem = Path::new(spec.executable)
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| package.clone());
    let install_dir = format!("usr/lib/{stem}");
    // Icons staged under share/icons/hicolor/... are named after the app id.
    let icon = files.iter().find_map(|file| {
        let rest = file.relative.strip_prefix("share/icons/")?;
        let name = rest.rsplit('/').next()?;
        Path::new(name)
            .file_stem()
            .map(|value| value.to_string_lossy().into_owned())
    });
    let install_size_kib: u64 = files
        .iter()
        .map(|file| file.bytes.len() as u64)
        .sum::<u64>()
        / 1024
        + 1;

    let mut data = Vec::new();
    {
        let encoder = GzEncoder::new(&mut data, Compression::default());
        let mut tar = tar::Builder::new(encoder);
        // Icons the staging step wrote under share/icons belong in the hicolor theme, not inside
        // the application directory: that is where desktop environments look them up.
        let mut placements: Vec<(String, &PayloadFile)> = Vec::new();
        let mut directories: BTreeSet<String> = BTreeSet::new();
        for file in &files {
            let path = target_path(&install_dir, &file.relative);
            let mut parent = path.rsplit_once('/').map(|(dir, _)| dir.to_owned());
            while let Some(dir) = parent {
                if !dir.is_empty() {
                    directories.insert(dir.clone());
                }
                parent = dir.rsplit_once('/').map(|(head, _)| head.to_owned());
            }
            placements.push((path, file));
        }
        let mut ordered: Vec<String> = directories.into_iter().collect();
        ordered.sort_by_key(|dir| (dir.matches('/').count(), dir.clone()));
        for directory in &ordered {
            append_dir(&mut tar, directory)?;
        }
        for (path, file) in &placements {
            append_file(&mut tar, path, &file.bytes, file.executable)?;
        }
        append_symlink(
            &mut tar,
            &format!("usr/bin/{stem}"),
            &format!("../lib/{stem}/{}", spec.executable),
        )?;
        append_file(
            &mut tar,
            &format!("usr/share/applications/{stem}.desktop"),
            desktop_entry(spec, &stem, icon.as_deref()).as_bytes(),
            false,
        )?;
        let encoder = tar.into_inner().context("finishing data.tar.gz")?;
        encoder.finish().context("finishing data.tar.gz")?;
    }

    let control = control_file(spec, &package, &version, arch, install_size_kib);
    let mut control_tar = Vec::new();
    {
        let encoder = GzEncoder::new(&mut control_tar, Compression::default());
        let mut tar = tar::Builder::new(encoder);
        append_file(&mut tar, "control", control.as_bytes(), false)?;
        let encoder = tar.into_inner().context("finishing control.tar.gz")?;
        encoder.finish().context("finishing control.tar.gz")?;
    }

    let mut archive = Vec::new();
    archive.extend_from_slice(b"!<arch>\n");
    append_ar_member(&mut archive, "debian-binary", b"2.0\n")?;
    append_ar_member(&mut archive, "control.tar.gz", &control_tar)?;
    append_ar_member(&mut archive, "data.tar.gz", &data)?;

    if let Some(parent) = spec.output.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(spec.output, &archive)
        .with_context(|| format!("writing {}", spec.output.display()))?;
    Ok(spec.output.to_path_buf())
}

/// Debian package names are lower case and may only use [a-z0-9+-.]; the app id is reverse DNS.
fn debian_name(app_id: &str) -> String {
    let name: String = app_id
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if name.is_empty() {
        "winia-app".to_owned()
    } else {
        name
    }
}

fn debian_version(version: &str) -> Result<String> {
    let value = version.trim();
    if value.is_empty() {
        bail!("app.version is empty");
    }
    if !value.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        bail!("app.version '{value}' must start with a digit for a Debian package");
    }
    Ok(value.to_owned())
}

fn debian_arch(arch: &str) -> Result<&'static str> {
    match arch {
        "x86_64" => Ok("amd64"),
        "aarch64" => Ok("arm64"),
        "x86" => Ok("i386"),
        "arm" => Ok("armhf"),
        other => bail!("unsupported Debian architecture '{other}'"),
    }
}

fn control_file(
    spec: &DebSpec<'_>,
    package: &str,
    version: &str,
    arch: &str,
    installed_size_kib: u64,
) -> String {
    let maintainer = if spec.publisher.trim().is_empty() {
        "Winia contributors <noreply@example.com>".to_owned()
    } else {
        spec.publisher.to_owned()
    };
    format!(
        "Package: {package}\nVersion: {version}\nArchitecture: {arch}\nMaintainer: {maintainer}\nInstalled-Size: {installed_size_kib}\nSection: utils\nPriority: optional\nDescription: {}\n A Winia desktop application.\n",
        spec.app_name
    )
}

fn desktop_entry(spec: &DebSpec<'_>, stem: &str, icon: Option<&str>) -> String {
    let icon_line = match icon {
        Some(name) => format!("Icon={name}\n"),
        None => String::new(),
    };
    format!(
        "[Desktop Entry]\nType=Application\nName={}\nComment={}\nExec={}\n{icon_line}Terminal=false\nCategories=Utility;\n",
        spec.app_name, spec.app_name, stem
    )
}

fn collect(payload: &Path, executable: &str) -> Result<Vec<PayloadFile>> {
    fn walk(
        root: &Path,
        dir: &Path,
        executable_name: &str,
        out: &mut Vec<PayloadFile>,
    ) -> Result<()> {
        for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                walk(root, &path, executable_name, out)?;
            } else if file_type.is_file() {
                let bytes =
                    std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                let executable = relative == executable_name
                    || relative.ends_with(".sh")
                    || relative.starts_with("bin/");
                out.push(PayloadFile {
                    relative,
                    bytes,
                    executable,
                });
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(payload, payload, executable, &mut files)?;
    files.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(files)
}

/// The staging tree keeps shared files under share/; the package moves them to /usr/share.
fn target_path(install_dir: &str, relative: &str) -> String {
    match relative.strip_prefix("share/") {
        Some(rest) => format!("usr/share/{rest}"),
        None => format!("{install_dir}/{relative}"),
    }
}

fn append_dir<W: Write>(tar: &mut tar::Builder<W>, path: &str) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Directory);
    header.set_mode(0o755);
    header.set_size(0);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_cksum();
    tar.append_data(&mut header, format!("./{path}/"), std::io::empty())
        .context("appending a directory")?;
    Ok(())
}

fn append_file<W: Write>(
    tar: &mut tar::Builder<W>,
    path: &str,
    bytes: &[u8],
    executable: bool,
) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_mode(if executable { 0o755 } else { 0o644 });
    header.set_size(bytes.len() as u64);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_cksum();
    tar.append_data(&mut header, format!("./{path}"), bytes)
        .context("appending a file")?;
    Ok(())
}

fn append_symlink<W: Write>(tar: &mut tar::Builder<W>, path: &str, target: &str) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_mode(0o777);
    header.set_size(0);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header
        .set_link_name(target)
        .context("setting the symlink target")?;
    header.set_cksum();
    tar.append_data(&mut header, format!("./{path}"), std::io::empty())
        .context("appending a symlink")?;
    Ok(())
}

/// ar header: 60 ASCII bytes, fields padded with spaces, data padded to an even length.
fn append_ar_member(out: &mut Vec<u8>, name: &str, data: &[u8]) -> Result<()> {
    if name.len() > 15 {
        bail!("ar member name '{name}' is too long");
    }
    let header = format!(
        "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}\u{60}\n",
        name,
        0,
        0,
        0,
        "100644",
        data.len()
    );
    if header.len() != 60 {
        bail!("internal error: ar header is {} bytes", header.len());
    }
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(b'\n');
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("deb-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let payload = root.join("payload");
        std::fs::create_dir_all(payload.join("assets")).unwrap();
        std::fs::write(payload.join("app"), b"binary").unwrap();
        std::fs::write(payload.join("assets/logo.png"), b"png").unwrap();
        root
    }

    fn spec<'a>(payload: &'a Path, output: &'a Path) -> DebSpec<'a> {
        DebSpec {
            app_name: "Winia Blank",
            app_version: "0.1.3",
            app_id: "com.example.winia_blank",
            publisher: "Winia contributors",
            executable: "app",
            payload,
            output,
            arch: "x86_64",
        }
    }

    #[test]
    fn writes_an_ar_archive_with_three_members() {
        let root = fixture("members");
        let out = root.join("app.deb");
        build(&spec(&root.join("payload"), &out)).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        assert!(bytes.starts_with(b"!<arch>\n"));
        let text = String::from_utf8_lossy(&bytes[..bytes.len().min(400)]);
        assert!(text.contains("debian-binary"));
        assert!(bytes.windows(11).any(|w| w == b"control.tar"));
        assert!(bytes.windows(9).any(|w| w == b"data.tar."));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn control_file_carries_the_package_identity() {
        let spec = DebSpec {
            app_name: "Winia Blank",
            app_version: "0.1.3",
            app_id: "com.example.winia_blank",
            publisher: "Winia contributors",
            executable: "app",
            payload: Path::new("."),
            output: Path::new("out.deb"),
            arch: "x86_64",
        };
        let control = control_file(&spec, "com.example.winia-blank", "0.1.3", "amd64", 42);
        assert!(control.contains("Package: com.example.winia-blank"));
        assert!(control.contains("Version: 0.1.3"));
        assert!(control.contains("Architecture: amd64"));
        assert!(control.contains("Installed-Size: 42"));
    }

    #[test]
    fn routes_shared_files_to_usr_share() {
        assert_eq!(
            target_path("usr/lib/demo", "share/icons/hicolor/48x48/apps/x.png"),
            "usr/share/icons/hicolor/48x48/apps/x.png"
        );
        assert_eq!(target_path("usr/lib/demo", "demo"), "usr/lib/demo/demo");
    }

    #[test]
    fn desktop_entry_references_the_theme_icon() {
        let spec = DebSpec {
            app_name: "Winia Blank",
            app_version: "0.1.3",
            app_id: "com.example.winia_blank",
            publisher: "Winia contributors",
            executable: "app",
            payload: Path::new("."),
            output: Path::new("out.deb"),
            arch: "x86_64",
        };
        let with_icon = desktop_entry(&spec, "app", Some("com.example.winia_blank"));
        assert!(with_icon.contains("Icon=com.example.winia_blank"));
        let without = desktop_entry(&spec, "app", None);
        assert!(!without.contains("Icon="));
    }

    #[test]
    fn maps_rust_architectures_to_debian_ones() {
        assert_eq!(debian_arch("x86_64").unwrap(), "amd64");
        assert_eq!(debian_arch("aarch64").unwrap(), "arm64");
        assert!(debian_arch("mips").is_err());
    }
}
