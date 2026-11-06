//! 'winia package' — assemble distributable Android artifacts.
//!
//! `apk` builds through the same pipeline as `winia build` and is copied to
//! `dist/` with a versioned name. `--split per-abi` builds one APK per ABI;
//! the default is a universal APK. `aab` runs the Gradle bundle task.
//! Every APK is checked for 16 KB ZIP alignment, the configured native ABIs,
//! version metadata, and (for debug builds) a valid signature.

use crate::cli::{BuildArgs, PackageArgs, SplitMode};
use crate::commands::build;
use crate::config::WiniaConfig;
use crate::platform::android::Abi;
use crate::platform::tools::{apksigner_command, build_tools_dir, executable};
use anyhow::{Context, Result, bail};
use flate2::Compression;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::copy;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use zip::write::SimpleFileOptions;

pub fn run(args: PackageArgs) -> Result<()> {
    let started = Instant::now();
    let cwd = std::env::current_dir().context("reading the current directory")?;
    let config_path = crate::config::find_config(&cwd)
        .ok_or_else(|| anyhow::anyhow!("winia.toml not found; run this inside a Winia project"))?;
    let root = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let config = WiniaConfig::load(&config_path)?;
    config.validate()?;

    let targets = resolve_targets(&root, &args)?;
    if targets.len() != 1 {
        bail!("package one target at a time");
    }
    let target = &targets[0];
    if target == "android" {
        package_android(&root, &config, &args, started)
    } else {
        package_desktop(&root, &config, &args, target, started)
    }
}

fn resolve_targets(root: &Path, args: &PackageArgs) -> Result<Vec<String>> {
    if args.target.is_empty() {
        if root.join("android").is_dir() {
            return Ok(vec!["android".to_owned()]);
        }
        return Ok(vec![build::host_target().to_owned()]);
    }
    let mut targets = Vec::new();
    for target in &args.target {
        match target.as_str() {
            "android" | "windows" | "macos" | "linux" => {
                if !targets.contains(target) {
                    targets.push(target.clone());
                }
            }
            other => bail!("unknown target '{other}'; expected windows, macos, linux, or android"),
        }
    }
    Ok(targets)
}

fn package_desktop(
    root: &Path,
    config: &WiniaConfig,
    args: &PackageArgs,
    target: &str,
    started: Instant,
) -> Result<()> {
    if target != build::host_target() {
        bail!(
            "packaging {target} on {} is not supported; run this on {target}",
            build::host_target()
        );
    }
    let out_dir = output_dir(root, args);
    let stem = artifact_stem(config);
    let version = config.app.version.clone();
    let arch = std::env::consts::ARCH;
    let formats = desktop_formats(target, config, args)?;
    let planned = planned_desktop_artifacts(&stem, &version, target, arch, &formats, &out_dir);

    if args.dry_run {
        return report(
            root,
            &planned,
            true,
            args.json,
            started.elapsed(),
            ReportMeta {
                target,
                formats: &formats,
                split: None,
                version: &version,
            },
        );
    }

    std::fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    if args.clean {
        clean_desktop_output(&out_dir, &stem, &version, target)?;
    }

    let profile = if args.release { "release" } else { "debug" };
    let context = crate::hooks::HookContext {
        target,
        profile,
        version: &version,
    };
    crate::hooks::run(root, config, crate::hooks::Phase::BeforePackage, &context)?;
    let built = build::build_all(
        root,
        config,
        &BuildArgs {
            target: vec![target.to_owned()],
            release: args.release,
            abi: Vec::new(),
            features: Vec::new(),
            json: args.json,
            out_dir: None,
        },
    )?;
    let executable = built
        .iter()
        .flat_map(|(_, artifacts)| artifacts)
        .find(|artifact| extension(&artifact.path) != Some("apk") && artifact.path.is_file())
        .ok_or_else(|| anyhow::anyhow!("the build produced no executable"))?;

    let stage = root
        .join("target")
        .join("winia-package")
        .join(format!("{stem}-{version}-{target}-{arch}"));
    if stage.exists() {
        std::fs::remove_dir_all(&stage).with_context(|| format!("removing {}", stage.display()))?;
    }
    std::fs::create_dir_all(&stage).with_context(|| format!("creating {}", stage.display()))?;
    let executable_name = executable
        .path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("executable has no file name"))?;
    std::fs::copy(&executable.path, stage.join(executable_name))
        .with_context(|| format!("copying {}", executable.path.display()))?;
    crate::assets::stage_desktop(root, config, &stage)?;
    let icons = crate::icons::ensure(root, config)?;
    if let Some(icons) = &icons {
        match target {
            "windows" => {
                std::fs::copy(&icons.windows_ico, stage.join("app.ico"))?;
            }
            "linux" => {
                copy_dir_all(&icons.linux_icons, &stage.join("share/icons"))?;
            }
            _ => {}
        }
    }

    // Installers need a single publisher; the first author is the closest thing to a vendor.
    let publisher = config
        .app
        .authors
        .first()
        .cloned()
        .unwrap_or_else(|| config.app.name.clone());
    let license = config.app.license.as_deref();
    let explicit = !args.format.is_empty();
    let mut produced = Vec::new();
    let mut built_formats = Vec::new();
    for format in &formats {
        let destination = desktop_artifact_path(&stem, &version, target, arch, format, &out_dir);
        match format.as_str() {
            "zip" if target == "macos" => {
                // A macOS zip without the .app bundle has no Info.plist and no identity.
                let bundle = macos_bundle_path(&out_dir, &stem, &version, target, arch);
                if !args.reuse_app || !bundle.exists() {
                    build_macos_app(&stage, &bundle, config, icons.as_ref(), &executable.path)?;
                }
                zip_directory(&bundle, &destination)?;
            }
            "zip" => zip_directory(&stage, &destination)?,
            "tar.gz" => tar_gz_directory(&stage, &destination)?,
            "app" => build_macos_app(
                &stage,
                &destination,
                config,
                icons.as_ref(),
                &executable.path,
            )?,
            "dmg" => {
                let bundle = macos_bundle_path(&out_dir, &stem, &version, target, arch);
                if !args.reuse_app || !bundle.exists() {
                    build_macos_app(&stage, &bundle, config, icons.as_ref(), &executable.path)?;
                }
                build_macos_dmg(&bundle, &destination, config)?;
            }
            "deb" => {
                let spec = winia_pack::deb::DebSpec {
                    app_name: &config.app.name,
                    app_version: &version,
                    app_id: &config.app.id,
                    publisher: &publisher,
                    license,
                    executable: executable_name
                        .to_str()
                        .ok_or_else(|| anyhow::anyhow!("the executable name is not valid UTF-8"))?,
                    payload: &stage,
                    output: &destination,
                    arch,
                };
                winia_pack::deb::build(&spec)?;
            }
            "msi" if !explicit && !winia_pack::msi::wix_available() => {
                eprintln!("note: the WiX toolset was not found; skipped msi");
                continue;
            }
            "nsis" if !explicit && winia_pack::nsis::find_makensis().is_err() => {
                eprintln!("note: makensis was not found; skipped nsis");
                continue;
            }
            "msi" => {
                let spec = winia_pack::msi::MsiSpec {
                    app_name: &config.app.name,
                    app_version: &version,
                    app_id: &config.app.id,
                    publisher: &publisher,
                    executable: executable_name
                        .to_str()
                        .ok_or_else(|| anyhow::anyhow!("the executable name is not valid UTF-8"))?,
                    payload: &stage,
                    output: &destination,
                    arch,
                };
                winia_pack::msi::build(&spec)?;
            }
            "nsis" => {
                let icon = icons
                    .as_ref()
                    .filter(|_| target == "windows")
                    .map(|_| Path::new("app.ico"));
                let spec = winia_pack::nsis::InstallerSpec {
                    app_name: &config.app.name,
                    app_version: &version,
                    app_id: &config.app.id,
                    publisher: &publisher,
                    license,
                    executable: executable_name
                        .to_str()
                        .ok_or_else(|| anyhow::anyhow!("the executable name is not valid UTF-8"))?,
                    payload: &stage,
                    output: &destination,
                    icon,
                    program_files_64: arch == "x86_64",
                };
                winia_pack::nsis::build(&spec)?;
            }
            other => bail!("unsupported desktop format '{other}'"),
        }
        produced.push(destination);
        built_formats.push(format.clone());
    }
    crate::hooks::run(root, config, crate::hooks::Phase::AfterPackage, &context)?;

    report(
        root,
        &produced,
        false,
        args.json,
        started.elapsed(),
        ReportMeta {
            target,
            formats: &built_formats,
            split: None,
            version: &version,
        },
    )
}

fn output_dir(root: &Path, args: &PackageArgs) -> PathBuf {
    match &args.out_dir {
        Some(path) if path.is_absolute() => path.clone(),
        Some(path) => root.join(path),
        None => root.join("dist"),
    }
}

fn desktop_formats(target: &str, config: &WiniaConfig, args: &PackageArgs) -> Result<Vec<String>> {
    let configured = match target {
        "windows" => &config.package.windows.formats,
        "macos" => &config.package.macos.formats,
        "linux" => &config.package.linux.formats,
        other => bail!("unknown desktop target '{other}'"),
    };
    let supported: &[&str] = match target {
        "windows" => &["zip", "nsis", "msi"],
        "macos" => &["app", "zip", "dmg"],
        "linux" => &["tar.gz", "deb"],
        _ => &[],
    };
    let requested = if args.format.is_empty() {
        configured
    } else {
        &args.format
    };
    let mut formats = Vec::new();
    let mut skipped = Vec::new();
    for format in requested {
        let format = format.to_lowercase();
        if supported.contains(&format.as_str()) {
            if !formats.contains(&format) {
                formats.push(format);
            }
        } else if args.format.is_empty() {
            skipped.push(format);
        } else {
            bail!(
                "format '{format}' is not supported for {target} yet; portable formats: {}",
                supported.join(", ")
            );
        }
    }
    if formats.is_empty() {
        formats.push(supported[0].to_owned());
    }
    if !skipped.is_empty() && !args.json {
        eprintln!(
            "note: formats not available yet; skipped: {}",
            skipped.join(", ")
        );
    }
    Ok(formats)
}

fn planned_desktop_artifacts(
    stem: &str,
    version: &str,
    target: &str,
    arch: &str,
    formats: &[String],
    out_dir: &Path,
) -> Vec<PathBuf> {
    formats
        .iter()
        .map(|format| desktop_artifact_path(stem, version, target, arch, format, out_dir))
        .collect()
}

/// Artifact path for a supported desktop format, or None when the format is not packaged.
pub(crate) fn release_artifact_path(
    stem: &str,
    version: &str,
    target: &str,
    arch: &str,
    format: &str,
    out_dir: &Path,
) -> Option<PathBuf> {
    let supported: &[&str] = match target {
        "windows" => &["zip", "nsis", "msi"],
        "macos" => &["app", "zip", "dmg"],
        "linux" => &["tar.gz", "deb"],
        _ => return None,
    };
    if !supported.contains(&format) {
        return None;
    }
    Some(desktop_artifact_path(
        stem, version, target, arch, format, out_dir,
    ))
}

fn desktop_artifact_path(
    stem: &str,
    version: &str,
    target: &str,
    arch: &str,
    format: &str,
    out_dir: &Path,
) -> PathBuf {
    if format == "nsis" {
        return out_dir.join(format!("{stem}-{version}-{target}-{arch}-setup.exe"));
    }
    out_dir.join(format!("{stem}-{version}-{target}-{arch}.{format}"))
}

fn macos_bundle_path(
    out_dir: &Path,
    stem: &str,
    version: &str,
    target: &str,
    arch: &str,
) -> PathBuf {
    out_dir.join(format!("{stem}-{version}-{target}-{arch}.app"))
}

/// Only artifacts the CLI can produce: a prefix match alone would delete notes, signatures or
/// unrelated files the user keeps next to dist.
fn is_desktop_artifact_name(name: &str) -> bool {
    if name.ends_with(".app") {
        return true;
    }
    ["zip", "exe", "msi", "deb", "dmg"]
        .iter()
        .any(|extension| name.ends_with(&format!(".{extension}")))
        || name.ends_with(".tar.gz")
}

fn clean_desktop_output(out_dir: &Path, stem: &str, version: &str, target: &str) -> Result<()> {
    if !out_dir.is_dir() {
        return Ok(());
    }
    let prefix = format!("{stem}-{version}-{target}-");
    for entry in
        std::fs::read_dir(out_dir).with_context(|| format!("reading {}", out_dir.display()))?
    {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with(&prefix) || !is_desktop_artifact_name(name) {
            continue;
        }
        if path.is_dir() {
            std::fs::remove_dir_all(&path)
                .with_context(|| format!("removing {}", path.display()))?;
        } else {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    }
    Ok(())
}

fn copy_dir_all(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)
        .with_context(|| format!("creating {}", destination.display()))?;
    for entry in
        std::fs::read_dir(source).with_context(|| format!("reading {}", source.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), &target).with_context(|| {
                format!("copying {} to {}", entry.path().display(), target.display())
            })?;
        }
    }
    Ok(())
}

fn collect_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_files(&entry.path(), files)?;
        } else if file_type.is_file() {
            files.push(entry.path());
        }
    }
    Ok(())
}

fn zip_directory(stage: &Path, archive: &Path) -> Result<()> {
    let file = File::create(archive).with_context(|| format!("creating {}", archive.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let root_name = stage
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "app".to_owned());
    let mut files = Vec::new();
    collect_files(stage, &mut files)?;
    for path in files {
        let relative = path.strip_prefix(stage).unwrap_or(&path);
        let name = format!(
            "{root_name}/{}",
            relative.to_string_lossy().replace('\\', "/")
        );
        let executable = {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(&path)?.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                false
            }
        };
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(if executable { 0o755 } else { 0o644 });
        zip.start_file(name, options)
            .with_context(|| format!("adding {} to zip", path.display()))?;
        let mut source =
            File::open(&path).with_context(|| format!("opening {}", path.display()))?;
        copy(&mut source, &mut zip)
            .with_context(|| format!("writing {} to zip", path.display()))?;
    }
    zip.finish().context("finishing zip archive")?;
    Ok(())
}

fn tar_gz_directory(stage: &Path, archive: &Path) -> Result<()> {
    let file = File::create(archive).with_context(|| format!("creating {}", archive.display()))?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = tar::Builder::new(encoder);
    let root_name = stage
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "app".to_owned());
    builder
        .append_dir_all(root_name, stage)
        .context("adding project directory to tar")?;
    let encoder = builder.into_inner().context("finishing tar")?;
    encoder.finish().context("finishing gzip")?;
    Ok(())
}

/// Wrap the .app bundle in a compressed disk image. hdiutil only exists on macOS.
fn build_macos_dmg(bundle: &Path, destination: &Path, config: &WiniaConfig) -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("dmg creation needs hdiutil, which only exists on macOS; run this on a Mac");
    }
    if destination.exists() {
        std::fs::remove_file(destination)
            .with_context(|| format!("removing {}", destination.display()))?;
    }
    let status = Command::new("hdiutil")
        .arg("create")
        .arg("-volname")
        .arg(&config.app.name)
        .arg("-srcfolder")
        .arg(bundle)
        .arg("-ov")
        .arg("-format")
        .arg("UDZO")
        .arg(destination)
        .stdout(Stdio::null())
        .status()
        .context("running hdiutil (macOS only)")?;
    if !status.success() {
        bail!("hdiutil failed with {status}");
    }
    Ok(())
}

fn build_macos_app(
    stage: &Path,
    bundle: &Path,
    config: &WiniaConfig,
    icons: Option<&crate::icons::IconSet>,
    executable: &Path,
) -> Result<()> {
    if bundle.exists() {
        std::fs::remove_dir_all(bundle)
            .with_context(|| format!("removing {}", bundle.display()))?;
    }
    let contents = bundle.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    std::fs::create_dir_all(&macos).with_context(|| format!("creating {}", macos.display()))?;
    std::fs::create_dir_all(&resources)
        .with_context(|| format!("creating {}", resources.display()))?;

    let executable_name = executable
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("executable has no file name"))?
        .to_owned();
    std::fs::copy(executable, macos.join(&executable_name))?;
    let assets = stage.join("assets");
    if assets.is_dir() {
        copy_dir_all(&assets, &resources.join("assets"))?;
    }
    let manifest = stage.join("assets.manifest.json");
    if manifest.is_file() {
        std::fs::copy(&manifest, resources.join("assets.manifest.json"))?;
    }
    let copyright_entry = match (config.app.license.as_deref(), config.app.authors.first()) {
        (Some(license), _) => format!(
            "  <key>NSHumanReadableCopyright</key><string>{}</string>\n",
            xml_escape(license)
        ),
        (None, Some(author)) => format!(
            "  <key>NSHumanReadableCopyright</key><string>{}</string>\n",
            xml_escape(author)
        ),
        (None, None) => String::new(),
    };
    let icon_entry = if let Some(icons) = icons {
        std::fs::copy(&icons.macos_icns, resources.join("AppIcon.icns"))?;
        "  <key>CFBundleIconFile</key><string>AppIcon</string>\n"
    } else {
        ""
    };

    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\"><dict>\n\
  <key>CFBundleName</key><string>{}</string>\n\
  <key>CFBundleIdentifier</key><string>{}</string>\n\
  <key>CFBundleExecutable</key><string>{}</string>\n\
  <key>CFBundlePackageType</key><string>APPL</string>\n\
  <key>CFBundleShortVersionString</key><string>{}</string>\n\
  <key>CFBundleVersion</key><string>{}</string>\n\
  <key>LSMinimumSystemVersion</key><string>11.0</string>\n\
{}{} </dict></plist>\n",
        xml_escape(&config.app.name),
        xml_escape(&config.app.id),
        xml_escape(&executable_name.to_string_lossy()),
        bundle_version(&config.app.version),
        bundle_version(&config.app.version),
        copyright_entry,
        icon_entry,
    );
    std::fs::write(contents.join("Info.plist"), plist)?;
    Ok(())
}

/// CFBundleVersion wants one to three period-separated integers; drop any semver suffix.
fn bundle_version(version: &str) -> String {
    let core = version.split(['-', '+']).next().unwrap_or("0.0.0");
    let parts: Vec<&str> = core.split('.').take(3).collect();
    if parts.len() == 3 && parts.iter().all(|part| part.parse::<u32>().is_ok()) {
        parts.join(".")
    } else {
        "0.0.0".to_owned()
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn package_android(
    root: &Path,
    config: &WiniaConfig,
    args: &PackageArgs,
    started: Instant,
) -> Result<()> {
    let android_dir = root.join("android");

    let formats: Vec<String> = if args.format.is_empty() {
        config.package.android.formats.clone()
    } else {
        args.format.clone()
    }
    .iter()
    .map(|format| format.to_lowercase())
    .collect();
    if formats.is_empty() {
        bail!(
            "no Android formats selected; pass --format apk,aab or set [package.android].formats"
        );
    }
    if let Some(unknown) = formats
        .iter()
        .find(|format| format.as_str() != "apk" && format.as_str() != "aab")
    {
        bail!("unsupported Android format '{unknown}'; expected apk or aab");
    }

    let abis: Vec<Abi> = config
        .android
        .abis
        .iter()
        .map(|abi| Abi::parse(abi))
        .collect::<Result<Vec<_>>>()?;
    if abis.is_empty() {
        bail!("no Android ABIs configured; set [android].abis");
    }

    let out_dir = match &args.out_dir {
        Some(path) if path.is_absolute() => path.clone(),
        Some(path) => root.join(path),
        None => root.join("dist"),
    };
    let stem = artifact_stem(config);
    let version = config.app.version.clone();
    let profile = if args.release { "release" } else { "debug" };
    let split = args.split.unwrap_or(SplitMode::Universal);
    if args.split.is_some() && !formats.iter().any(|format| format == "apk") && !args.json {
        eprintln!(
            "warning: --split only affects the apk format; ignoring it for the requested formats"
        );
    }
    let planned = planned_artifacts(&stem, &version, &formats, split, &abis, &out_dir);

    if args.dry_run {
        return report(
            root,
            &planned,
            true,
            args.json,
            started.elapsed(),
            ReportMeta {
                target: "android",
                formats: &formats,
                split: Some(split),
                version: &version,
            },
        );
    }

    std::fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    if args.clean {
        clean_output_dir(&out_dir, &stem, &version)?;
    } else if has_package_artifacts(&out_dir, &stem, &version) && !args.json {
        println!(
            "note: {} already contains files; pass --clean to remove stale artifacts",
            out_dir.display()
        );
    }

    let context = crate::hooks::HookContext {
        target: "android",
        profile,
        version: &version,
    };
    crate::hooks::run(root, config, crate::hooks::Phase::BeforePackage, &context)?;
    let mut produced = Vec::new();

    if formats.iter().any(|format| format == "apk") {
        match split {
            SplitMode::Universal => {
                let built = build::build_all(root, config, &build_args(args, Vec::new()))?;
                let apk = find_apk(&built)?;
                let destination = out_dir.join(format!("{stem}-{version}-android-universal.apk"));
                copy_artifact(&apk, &destination)?;
                verify_apk(
                    &destination,
                    &abis,
                    args.release,
                    config.android.version_code,
                    &version,
                    args.json,
                )?;
                produced.push(destination);
            }
            SplitMode::PerAbi => {
                for abi in &abis {
                    let built = build::build_all(
                        root,
                        config,
                        &build_args(args, vec![abi.as_str().to_owned()]),
                    )?;
                    let apk = find_apk(&built)?;
                    let destination =
                        out_dir.join(format!("{stem}-{version}-android-{}.apk", abi.as_str()));
                    copy_artifact(&apk, &destination)?;
                    verify_apk(
                        &destination,
                        &[*abi],
                        args.release,
                        config.android.version_code,
                        &version,
                        args.json,
                    )?;
                    produced.push(destination);
                }
            }
        }
    }

    if formats.iter().any(|format| format == "aab") {
        build::build_android_libraries(root, config, &build_args(args, Vec::new()))?;
        let task = if args.release {
            "bundleRelease"
        } else {
            "bundleDebug"
        };
        build::run_gradle_task(
            &android_dir,
            task,
            &build::gradle_properties(config),
            args.json,
        )?;
        let bundle_dir = android_dir.join("app/build/outputs/bundle").join(profile);
        let mut bundle = bundle_dir.join(format!("app-{profile}.aab"));
        if !bundle.is_file() {
            let mut candidates: Vec<PathBuf> = std::fs::read_dir(&bundle_dir)
                .map(|entries| {
                    entries
                        .flatten()
                        .map(|entry| entry.path())
                        .filter(|path| extension(path) == Some("aab"))
                        .collect()
                })
                .unwrap_or_default();
            candidates.sort_by_key(|path| {
                std::fs::metadata(path)
                    .and_then(|metadata| metadata.modified())
                    .ok()
            });
            bundle = candidates.pop().ok_or_else(|| {
                anyhow::anyhow!(
                    "Gradle finished but no AAB was found in {}",
                    bundle_dir.display()
                )
            })?;
        }
        let destination = out_dir.join(format!("{stem}-{version}-android.aab"));
        copy_artifact(&bundle, &destination)?;
        validate_aab(&destination, args.json)?;
        produced.push(destination);
    }
    crate::hooks::run(root, config, crate::hooks::Phase::AfterPackage, &context)?;

    report(
        root,
        &produced,
        false,
        args.json,
        started.elapsed(),
        ReportMeta {
            target: "android",
            formats: &formats,
            split: Some(split),
            version: &version,
        },
    )
}

fn planned_artifacts(
    stem: &str,
    version: &str,
    formats: &[String],
    split: SplitMode,
    abis: &[Abi],
    out_dir: &Path,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if formats.iter().any(|format| format == "apk") {
        match split {
            SplitMode::Universal => {
                paths.push(out_dir.join(format!("{stem}-{version}-android-universal.apk")))
            }
            SplitMode::PerAbi => {
                for abi in abis {
                    paths.push(
                        out_dir.join(format!("{stem}-{version}-android-{}.apk", abi.as_str())),
                    );
                }
            }
        }
    }
    if formats.iter().any(|format| format == "aab") {
        paths.push(out_dir.join(format!("{stem}-{version}-android.aab")));
    }
    paths
}

fn build_args(args: &PackageArgs, abis: Vec<String>) -> BuildArgs {
    BuildArgs {
        target: vec!["android".to_owned()],
        release: args.release,
        abi: abis,
        features: Vec::new(),
        json: args.json,
        out_dir: None,
    }
}

fn find_apk(built: &[(String, Vec<build::Artifact>)]) -> Result<PathBuf> {
    built
        .iter()
        .flat_map(|(_, artifacts)| artifacts.iter())
        .find(|artifact| extension(&artifact.path) == Some("apk"))
        .map(|artifact| artifact.path.clone())
        .ok_or_else(|| anyhow::anyhow!("the build produced no APK"))
}

fn copy_artifact(source: &Path, destination: &Path) -> Result<()> {
    if source == destination {
        return Ok(());
    }
    std::fs::copy(source, destination)
        .with_context(|| format!("copying {} to {}", source.display(), destination.display()))?;
    Ok(())
}

fn clean_output_dir(out_dir: &Path, stem: &str, version: &str) -> Result<()> {
    if !out_dir.is_dir() {
        return Ok(());
    }
    for entry in
        std::fs::read_dir(out_dir).with_context(|| format!("reading {}", out_dir.display()))?
    {
        let path = entry?.path();
        if path.is_file() && is_package_artifact(&path, stem, version) {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    }
    Ok(())
}

fn is_package_artifact(path: &Path, stem: &str, version: &str) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let base = format!("{stem}-{version}-android");
    (name.starts_with(&format!("{base}-")) || name.starts_with(&format!("{base}.")))
        && (name.ends_with(".apk") || name.ends_with(".aab"))
}

fn has_package_artifacts(directory: &Path, stem: &str, version: &str) -> bool {
    std::fs::read_dir(directory)
        .map(|entries| {
            entries
                .flatten()
                .any(|entry| is_package_artifact(&entry.path(), stem, version))
        })
        .unwrap_or(false)
}

fn verify_apk(
    apk: &Path,
    abis: &[Abi],
    release: bool,
    version_code: u32,
    version_name: &str,
    json: bool,
) -> Result<()> {
    let Some(build_tools) = build_tools_dir() else {
        if !json {
            eprintln!("warning: Android build-tools not found; skipped APK verification");
        }
        return Ok(());
    };

    let zipalign = build_tools.join(executable("zipalign"));
    if zipalign.is_file() {
        let output = Command::new(&zipalign)
            .args(["-c", "-P", "16", "4"])
            .arg(apk)
            .output()
            .with_context(|| format!("running {}", zipalign.display()))?;
        if !output.status.success() {
            bail!(
                "{} is not 16 KB aligned: {}",
                apk.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    } else if !json {
        eprintln!("warning: zipalign not found; skipped alignment check");
    }

    let aapt2 = build_tools.join(executable("aapt2"));
    if aapt2.is_file() {
        let output = Command::new(&aapt2)
            .args(["dump", "badging"])
            .arg(apk)
            .output()
            .with_context(|| format!("running {}", aapt2.display()))?;
        if !output.status.success() {
            bail!(
                "aapt2 failed on {}: {}",
                apk.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let native = text
            .lines()
            .find(|line| line.starts_with("native-code:"))
            .unwrap_or("native-code: (none)");
        for abi in abis {
            let needle = format!("'{}'", abi.as_str());
            if !native.contains(&needle) {
                bail!(
                    "{} does not contain native code for {} ({native})",
                    apk.display(),
                    abi.as_str()
                );
            }
        }
        match badging_version(&text) {
            Some((actual_code, actual_name)) => {
                if actual_code != Some(version_code) {
                    bail!(
                        "{} reports versionCode {:?}, expected {}",
                        apk.display(),
                        actual_code,
                        version_code
                    );
                }
                if actual_name.as_deref() != Some(version_name) {
                    bail!(
                        "{} reports versionName {:?}, expected {}",
                        apk.display(),
                        actual_name,
                        version_name
                    );
                }
            }
            None => {
                if !json {
                    eprintln!(
                        "warning: could not read version metadata from {}",
                        apk.display()
                    );
                }
            }
        }
    } else if !json {
        eprintln!("warning: aapt2 not found; skipped ABI and version checks");
    }

    let launcher = build_tools.join(if cfg!(windows) {
        "apksigner.bat"
    } else {
        "apksigner"
    });
    if launcher.is_file() || build_tools.join("lib").join("apksigner.jar").is_file() {
        let mut command = apksigner_command(&build_tools)?;
        command.args(["verify", "--print-certs"]).arg(apk);
        let output = command.output().context("running apksigner")?;
        if !output.status.success() {
            if release {
                if !json {
                    eprintln!(
                        "warning: {} is unsigned; run 'winia sign --target android' before distributing",
                        apk.display()
                    );
                }
            } else {
                bail!(
                    "signature verification failed for {}: {}",
                    apk.display(),
                    String::from_utf8_lossy(&output.stdout).trim()
                );
            }
        }
    } else if !json {
        eprintln!("warning: apksigner not found; skipped signature check");
    }
    Ok(())
}

fn validate_aab(aab: &Path, json: bool) -> Result<()> {
    let Some(mut command) = bundletool_command() else {
        if !json {
            eprintln!("warning: bundletool not found; skipped AAB validation");
        }
        return Ok(());
    };
    let output = command
        .args(["validate", "--bundle"])
        .arg(aab)
        .output()
        .context("running bundletool")?;
    if !output.status.success() {
        bail!(
            "bundletool rejected {}: {}",
            aab.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn bundletool_command() -> Option<Command> {
    if let Some(value) = std::env::var_os("BUNDLETOOL_JAR").filter(|value| !value.is_empty()) {
        let jar = PathBuf::from(value);
        if jar.is_file() {
            let mut command = Command::new("java");
            command.arg("-jar").arg(jar);
            return Some(command);
        }
    }
    if Command::new("bundletool")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
    {
        return Some(Command::new("bundletool"));
    }
    None
}

pub(crate) fn artifact_stem(config: &WiniaConfig) -> String {
    let stem: String = config
        .app
        .name
        .to_lowercase()
        .replace(' ', "-")
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || *character == '-' || *character == '_'
        })
        .collect();
    if stem.is_empty() {
        "app".to_owned()
    } else {
        stem
    }
}

fn extension(path: &Path) -> Option<&str> {
    path.extension().and_then(|extension| extension.to_str())
}

fn attribute(line: &str, key: &str) -> Option<String> {
    let marker = format!("{key}='");
    let start = line.find(&marker)? + marker.len();
    let rest = &line[start..];
    let end = rest.find('\'')?;
    Some(rest[..end].to_owned())
}

fn badging_version(text: &str) -> Option<(Option<u32>, Option<String>)> {
    let line = text.lines().find(|line| line.starts_with("package: "))?;
    let code = attribute(line, "versionCode").and_then(|value| value.parse().ok());
    let name = attribute(line, "versionName");
    Some((code, name))
}

struct ReportMeta<'a> {
    target: &'a str,
    formats: &'a [String],
    split: Option<SplitMode>,
    version: &'a str,
}

fn report(
    root: &Path,
    artifacts: &[PathBuf],
    dry_run: bool,
    json: bool,
    elapsed: Duration,
    meta: ReportMeta<'_>,
) -> Result<()> {
    let hashes: Vec<Option<String>> = if dry_run {
        vec![None; artifacts.len()]
    } else {
        artifacts.iter().map(|path| sha256_of(path)).collect()
    };
    if !dry_run && let Some(directory) = artifacts.first().and_then(|path| path.parent()) {
        write_checksums(directory, artifacts, &hashes)?;
    }
    if json {
        let values: Vec<_> = artifacts
            .iter()
            .zip(hashes.iter())
            .map(|(path, hash)| {
                let bytes = if dry_run {
                    0
                } else {
                    std::fs::metadata(path)
                        .map(|metadata| metadata.len())
                        .unwrap_or(0)
                };
                let mut entry = serde_json::json!({
                    "path": display_relative(root, path),
                    "bytes": bytes,
                });
                if let Some(hash) = hash {
                    entry["sha256"] = serde_json::json!(hash);
                }
                entry
            })
            .collect();
        let split = meta.split.map(|split| match split {
            SplitMode::Universal => "universal",
            SplitMode::PerAbi => "per-abi",
        });
        let mut value = serde_json::json!({
            "target": meta.target,
            "version": meta.version,
            "formats": meta.formats,
            "dry_run": dry_run,
            "artifacts": values,
            "duration_ms": elapsed.as_millis(),
        });
        if let Some(split) = split {
            value["split"] = serde_json::json!(split);
        }
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    println!(
        "{}",
        if dry_run {
            "Would create:"
        } else {
            "Packaged:"
        }
    );
    for artifact in artifacts {
        if dry_run {
            println!("  {}", display_relative(root, artifact));
        } else {
            let bytes = std::fs::metadata(artifact)
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            println!(
                "  {} ({:.1} MB)",
                display_relative(root, artifact),
                bytes as f64 / (1024.0 * 1024.0)
            );
        }
    }
    Ok(())
}

/// SHA-256 of a produced artifact, or None when it cannot be read.
fn sha256_of(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(format!("{:x}", Sha256::digest(&bytes)))
}

/// Read an existing SHA256SUMS into memory (name -> hash).
fn read_checksums(path: &Path) -> std::collections::BTreeMap<String, String> {
    let mut entries = std::collections::BTreeMap::new();
    if let Ok(existing) = std::fs::read_to_string(path) {
        for line in existing.lines() {
            if let Some((hash, name)) = line.split_once("  ") {
                entries.insert(name.trim().to_owned(), hash.trim().to_owned());
            }
        }
    }
    entries
}

fn write_checksums_file(
    path: &Path,
    entries: &std::collections::BTreeMap<String, String>,
) -> Result<()> {
    let mut text = String::new();
    for (name, hash) in entries {
        text.push_str(&checksum_line(hash, name));
    }
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Refresh the SHA256SUMS entries for files that were rewritten after packaging (signing).
/// Does nothing when the directory has no checksum file, so standalone signing stays optional.
pub(crate) fn update_checksums(files: &[PathBuf]) -> Result<()> {
    let Some(directory) = files.first().and_then(|file| file.parent()) else {
        return Ok(());
    };
    let path = directory.join("SHA256SUMS");
    if !path.is_file() {
        return Ok(());
    }
    let mut entries = read_checksums(&path);
    for file in files {
        let (Some(hash), Some(name)) = (
            sha256_of(file),
            file.file_name().and_then(|name| name.to_str()),
        ) else {
            continue;
        };
        entries.insert(name.to_owned(), hash);
    }
    write_checksums_file(&path, &entries)
}

/// Write SHA256SUMS next to the artifacts, in the format sha256sum -c expects.
fn write_checksums(
    directory: &Path,
    artifacts: &[PathBuf],
    hashes: &[Option<String>],
) -> Result<()> {
    // Merge with the existing file: dist usually holds artifacts from several runs, and replacing
    // the whole file would drop entries for files that are still there.
    let path = directory.join("SHA256SUMS");
    let mut entries = read_checksums(&path);
    for (path, hash) in artifacts.iter().zip(hashes.iter()) {
        let (Some(hash), Some(name)) = (hash, path.file_name().and_then(|name| name.to_str()))
        else {
            continue;
        };
        entries.insert(name.to_owned(), hash.clone());
    }
    write_checksums_file(&path, &entries)
}

fn checksum_line(hash: &str, name: &str) -> String {
    format!("{hash}  {name}\n")
}

fn display_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_only_deletes_artifacts_it_produces() {
        assert!(is_desktop_artifact_name("app-0.1.0-windows-x86_64.zip"));
        assert!(is_desktop_artifact_name("app-0.1.0-macos-x86_64.app"));
        assert!(is_desktop_artifact_name("app-0.1.0-linux-x86_64.tar.gz"));
        assert!(!is_desktop_artifact_name(
            "app-0.1.0-windows-x86_64-CHANGELOG.txt"
        ));
        assert!(!is_desktop_artifact_name(
            "app-0.1.0-windows-x86_64.zip.asc"
        ));
    }

    #[test]
    fn signing_refreshes_existing_checksums() {
        let dir = std::env::temp_dir().join(format!("winia-sums-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let artifact = dir.join("app.zip");
        std::fs::write(&artifact, b"after").unwrap();
        std::fs::write(dir.join("SHA256SUMS"), "0000  app.zip\n1111  other.zip\n").unwrap();
        update_checksums(std::slice::from_ref(&artifact)).unwrap();
        let text = std::fs::read_to_string(dir.join("SHA256SUMS")).unwrap();
        assert!(!text.contains("0000"), "{text}");
        assert!(text.contains("1111  other.zip"), "{text}");
        let expected = sha256_of(&artifact).unwrap();
        assert!(text.contains(&expected), "{text}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn checksum_lines_match_the_sha256sum_format() {
        assert_eq!(
            checksum_line("abc123", "app-1.0.0.zip"),
            "abc123  app-1.0.0.zip\n"
        );
    }

    #[test]
    fn artifact_names_are_lowercase_and_safe() {
        let mut config = WiniaConfig::default();
        config.app.name = "My App!".to_owned();
        assert_eq!(artifact_stem(&config), "my-app");
    }

    #[test]
    fn desktop_artifact_names_include_target_and_arch() {
        let out = Path::new("dist");
        assert_eq!(
            desktop_artifact_path("demo", "1.0.0", "windows", "x86_64", "zip", out),
            PathBuf::from("dist/demo-1.0.0-windows-x86_64.zip")
        );
        assert_eq!(
            desktop_artifact_path("demo", "1.0.0", "linux", "aarch64", "tar.gz", out),
            PathBuf::from("dist/demo-1.0.0-linux-aarch64.tar.gz")
        );
        assert_eq!(
            desktop_artifact_path("demo", "1.0.0", "macos", "aarch64", "app", out),
            PathBuf::from("dist/demo-1.0.0-macos-aarch64.app")
        );
    }

    #[test]
    fn planned_names_follow_the_format_and_split() {
        let abis = [Abi::Arm64V8a, Abi::X86_64];
        let out = Path::new("dist");
        let universal = planned_artifacts(
            "demo",
            "0.1.0",
            &["apk".to_owned()],
            SplitMode::Universal,
            &abis,
            out,
        );
        assert_eq!(
            universal,
            vec![PathBuf::from("dist/demo-0.1.0-android-universal.apk")]
        );

        let split = planned_artifacts(
            "demo",
            "0.1.0",
            &["apk".to_owned(), "aab".to_owned()],
            SplitMode::PerAbi,
            &abis,
            out,
        );
        assert_eq!(
            split,
            vec![
                PathBuf::from("dist/demo-0.1.0-android-arm64-v8a.apk"),
                PathBuf::from("dist/demo-0.1.0-android-x86_64.apk"),
                PathBuf::from("dist/demo-0.1.0-android.aab"),
            ]
        );
    }

    #[test]
    fn reads_version_metadata_from_aapt_badging() {
        let line = "package: name='com.example.demo' versionCode='7' versionName='0.2.0' platformBuildVersionName='16'";
        assert_eq!(
            badging_version(line),
            Some((Some(7), Some("0.2.0".to_owned())))
        );
    }

    #[test]
    fn only_matching_artifacts_are_cleaned() {
        let stem = "demo";
        let version = "0.1.0";
        assert!(is_package_artifact(
            Path::new("demo-0.1.0-android-universal.apk"),
            stem,
            version
        ));
        assert!(is_package_artifact(
            Path::new("demo-0.1.0-android-arm64-v8a.apk"),
            stem,
            version
        ));
        assert!(is_package_artifact(
            Path::new("demo-0.1.0-android.aab"),
            stem,
            version
        ));
        assert!(!is_package_artifact(Path::new("notes.txt"), stem, version));
        assert!(!is_package_artifact(
            Path::new("demo-0.2.0-android-universal.apk"),
            stem,
            version
        ));
    }
}
