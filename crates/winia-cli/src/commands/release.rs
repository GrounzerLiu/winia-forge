//! 'winia release' — bump, build, sign, tag, and publish an Android release.
//!
//! The release is deliberately conservative: it refuses a dirty worktree unless
//! `--allow-dirty` is passed, commits the version bump before tagging, and only
//! uploads when `--track github` is requested. Play track uploads are not
//! implemented yet.

use crate::cli::{Bump, PackageArgs, ReleaseArgs, SignArgs};
use crate::commands::{package, sign};
use crate::config::WiniaConfig;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

pub fn run(args: ReleaseArgs) -> Result<()> {
    let started = Instant::now();
    let cwd = std::env::current_dir().context("reading the current directory")?;
    let config_path = crate::config::find_config(&cwd)
        .ok_or_else(|| anyhow::anyhow!("winia.toml not found; run this inside a Winia project"))?;
    let root = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let mut config = WiniaConfig::load(&config_path)?;
    config.validate()?;

    let targets = if args.target.is_empty() {
        vec!["android".to_owned()]
    } else {
        args.target.clone()
    };
    for target in &targets {
        match target.as_str() {
            "windows" | "macos" | "linux" | "android" => {}
            other => bail!(
                "unknown release target '{other}'; expected windows, macos, linux, or android"
            ),
        }
    }
    let android = targets.iter().any(|target| target == "android");
    let track = match args.track.as_deref() {
        None => None,
        Some("github") => Some("github"),
        Some(other) => {
            bail!("release track '{other}' is not implemented yet; use --track github or omit it")
        }
    };
    let push = args.push || track == Some("github");
    if track.is_some() && args.no_tag {
        bail!("GitHub releases require a tag; remove --no-tag");
    }

    let current = config.app.version.clone();
    let version = match args.bump {
        Some(bump) => bump_version(&current, bump)?,
        None => current.clone(),
    };
    let tag = format!("v{version}");
    let stem = package::artifact_stem(&config);
    let out_dir = root.join("dist");
    let arch = std::env::consts::ARCH;
    let planned: Vec<PathBuf> = targets
        .iter()
        .flat_map(|target| {
            planned_artifacts(
                &stem,
                &version,
                target,
                arch,
                &formats_for(&config, target),
                &out_dir,
            )
        })
        .collect();
    let notes = release_notes(&root)?;

    if !args.no_tag
        && git_status(
            &root,
            &["rev-parse", "-q", "--verify", &format!("refs/tags/{tag}")],
        )?
    {
        bail!("tag {tag} already exists");
    }
    if !args.allow_dirty {
        let status = git(&root, &["status", "--porcelain"])?;
        if !status.trim().is_empty() {
            bail!("working tree is dirty; commit the changes or pass --allow-dirty");
        }
    }

    if args.dry_run {
        if args.json {
            let values: Vec<_> = planned
                .iter()
                .map(|path| {
                    serde_json::json!({
                        "name": path.file_name().and_then(|name| name.to_str()).unwrap_or_default(),
                        "bytes": 0,
                    })
                })
                .collect();
            let value = serde_json::json!({
                "targets": targets.clone(),
                "dry_run": true,
                "version": version,
                "tag": if args.no_tag { serde_json::Value::Null } else { serde_json::json!(tag) },
                "pushed": push,
                "track": track,
                "artifacts": values,
                "notes": notes,
                "duration_ms": 0,
            });
            println!("{}", serde_json::to_string_pretty(&value)?);
            return Ok(());
        }
        println!("Release plan");
        if version == current {
            println!("  version: {version}");
        } else {
            println!("  version: {current} -> {version}");
        }
        println!("  tag: {}", if args.no_tag { "(none)" } else { &tag });
        println!("  track: {}", track.unwrap_or("none"));
        println!("  push: {push}");
        println!("  artifacts:");
        for artifact in &planned {
            println!("    {}", display_relative(&root, artifact));
        }
        println!();
        println!("{notes}");
        return Ok(());
    }

    validate_signing_environment()?;
    if track == Some("github") {
        preflight_github(&root, &tag)?;
    }

    if args.bump.is_some() {
        config.app.version = version.clone();
        config.android.version_code = config.android.version_code.saturating_add(1);
        // Edit the two keys in place: save() would drop comments and keys this version does not know.
        crate::config::set_release_versions(
            &config_path,
            &config.app.version,
            config.android.version_code,
        )?;
        let mut files = vec!["winia.toml".to_owned()];
        if update_cargo_version(&root, &version)? {
            files.push("Cargo.toml".to_owned());
            // Refresh Cargo.lock so the tagged commit builds with --locked.
            let refreshed = std::process::Command::new("cargo")
                .args(["metadata", "--format-version", "1", "--no-deps"])
                .current_dir(&root)
                .output()
                .context("refreshing Cargo.lock after the version bump")?;
            if !refreshed.status.success() {
                bail!(
                    "cargo metadata failed after the version bump: {}",
                    String::from_utf8_lossy(&refreshed.stderr).trim()
                );
            }
            if root.join("Cargo.lock").is_file() {
                files.push("Cargo.lock".to_owned());
            }
        }
        let mut add = vec!["add".to_owned()];
        add.extend(files);
        let add_args: Vec<&str> = add.iter().map(String::as_str).collect();
        git(&root, &add_args)?;
        git(&root, &["commit", "-m", &format!("chore(release): {tag}")])?;
    }

    // Anything already in dist before this point was produced by an earlier run (or copied in by
    // hand); it must not end up in this release just because the name matches.
    let run_started = std::time::SystemTime::now();

    for target in &targets {
        let mut formats = available_formats(&formats_for(&config, target));
        // An .app is a directory, so a macOS release needs a zip to publish the bundle.
        if target == "macos"
            && formats.iter().any(|format| format == "app")
            && !formats.iter().any(|format| format == "zip")
        {
            eprintln!("note: adding zip to the macOS release so the .app bundle can be published");
            formats.push("zip".to_owned());
        }
        // A bundle packed before signing would stay unsigned, so build those formats after signing.
        let deferred: Vec<String> = if target == "macos" && macos_identity_configured() {
            formats
                .iter()
                .filter(|format| matches!(format.as_str(), "dmg" | "zip"))
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        let first: Vec<String> = if deferred.is_empty() {
            formats.clone()
        } else {
            formats
                .iter()
                .filter(|format| !deferred.contains(format))
                .cloned()
                .collect()
        };
        if args.json {
            run_package_json(&root, target, &first, &out_dir)?;
        } else {
            package::run(PackageArgs {
                target: vec![target.clone()],
                format: first.clone(),
                release: true,
                out_dir: Some(out_dir.clone()),
                split: None,
                json: false,
                dry_run: false,
                clean: false,
                reuse_app: false,
            })?;
        }
        if !deferred.is_empty() {
            let bundle = out_dir.join(format!("{stem}-{version}-macos-{arch}.app"));
            if bundle.exists() {
                super::sign_macos::run(
                    &root,
                    &root,
                    &SignArgs {
                        target: vec!["macos".to_owned()],
                        keystore: None,
                        apk: None,
                        file: vec![bundle],
                        alias: None,
                        in_place: true,
                        dry_run: false,
                        release: true,
                        json: false,
                    },
                )?;
            }
            package::run(PackageArgs {
                target: vec![target.clone()],
                format: deferred.clone(),
                release: true,
                out_dir: Some(out_dir.clone()),
                split: None,
                json: false,
                dry_run: false,
                clean: false,
                // The bundle was just signed; rebuilding it would drop the signature.
                reuse_app: true,
            })?;
        }
    }

    let scanned = version_artifacts(&out_dir, &stem, &version)?;
    let mut artifacts: Vec<PathBuf> = scanned
        .iter()
        .filter(|path| is_fresh(path, run_started))
        .cloned()
        .collect();
    let stale: Vec<&PathBuf> = scanned
        .iter()
        .filter(|path| !is_fresh(path, run_started))
        .collect();
    if !stale.is_empty() {
        eprintln!(
            "warning: ignoring {} file(s) in {} that this run did not produce: {}",
            stale.len(),
            out_dir.display(),
            stale
                .iter()
                .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if artifacts.is_empty() {
        bail!("packaging produced no artifacts for {version}");
    }

    if android && keystore_configured() {
        let unsigned_apk = artifacts
            .iter()
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.ends_with("-universal.apk") && !name.ends_with("-signed.apk"))
                    .unwrap_or(false)
            })
            .cloned();
        if let Some(apk) = unsigned_apk {
            if args.json {
                run_sign_json(&root, &apk)?;
            } else {
                sign::run(SignArgs {
                    target: vec!["android".to_owned()],
                    keystore: None,
                    apk: Some(apk.clone()),
                    file: Vec::new(),
                    alias: None,
                    in_place: false,
                    dry_run: false,
                    release: false,
                    json: false,
                })?;
            }
            let signed = signed_path(&apk);
            if signed.is_file() && !artifacts.contains(&signed) {
                artifacts.push(signed);
            }
        }
    } else if android && !artifacts.is_empty() {
        eprintln!("warning: no Android keystore configured; release artifacts are unsigned");
    }
    if windows_cert_configured() {
        let files: Vec<PathBuf> = artifacts
            .iter()
            .filter(|path| matches!(extension(path), Some("exe" | "msi")))
            .cloned()
            .collect();
        if !files.is_empty() {
            super::sign_windows::run(
                &root,
                &root,
                &SignArgs {
                    target: vec!["windows".to_owned()],
                    keystore: None,
                    apk: None,
                    file: files,
                    alias: None,
                    in_place: true,
                    dry_run: false,
                    release: true,
                    json: false,
                },
            )?;
        }
    } else if artifacts
        .iter()
        .any(|path| matches!(extension(path), Some("exe" | "msi")))
    {
        eprintln!("warning: no Windows certificate configured; desktop artifacts are unsigned");
    }

    let mut artifacts = select_release_artifacts(&artifacts);
    if let Some(checksums) = write_release_checksums(&out_dir, &artifacts)? {
        artifacts.push(checksums);
    }

    if !args.no_tag {
        git(&root, &["tag", "-a", &tag, "-m", &tag])?;
    }
    if push {
        push_release(&root, if args.no_tag { None } else { Some(&tag) })?;
    }

    // Write latest.json before publishing so the release carries it.
    let manifest = write_manifest(&out_dir, &version, &tag, &artifacts)?;

    if track == Some("github") {
        let mut assets = artifacts.clone();
        assets.push(manifest.clone());
        upload_github(&root, &tag, &notes, &assets)?;
    }

    if args.json {
        let values: Vec<_> = artifacts
            .iter()
            .map(|path| {
                let bytes = std::fs::metadata(path)
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                serde_json::json!({
                    "name": path.file_name().and_then(|name| name.to_str()).unwrap_or_default(),
                    "bytes": bytes,
                })
            })
            .collect();
        let value = serde_json::json!({
            "targets": targets.clone(),
            "version": version,
            "tag": if args.no_tag { serde_json::Value::Null } else { serde_json::json!(tag) },
            "pushed": push,
            "published": track.is_some(),
            "artifacts": values,
            "notes": notes,
            "duration_ms": started.elapsed().as_millis(),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }

    println!(
        "Released {tag} ({} artifacts) in {:.1}s",
        artifacts.len(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

fn run_self_json(root: &Path, args: &[String]) -> Result<serde_json::Value> {
    let executable = std::env::current_exe().context("locating the winia executable")?;
    let output = Command::new(executable)
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| format!("running winia {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "winia {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::from_slice(&output.stdout).context("parsing winia JSON output")
}

fn run_package_json(root: &Path, target: &str, formats: &[String], out_dir: &Path) -> Result<()> {
    let mut args = vec![
        "package".to_owned(),
        "--target".to_owned(),
        target.to_owned(),
        "--release".to_owned(),
        "--out-dir".to_owned(),
        out_dir.display().to_string(),
        "--json".to_owned(),
    ];
    if !formats.is_empty() {
        args.push("--format".to_owned());
        args.push(formats.join(","));
    }
    run_self_json(root, &args)?;
    Ok(())
}

fn run_sign_json(root: &Path, apk: &Path) -> Result<()> {
    let args = vec![
        "sign".to_owned(),
        "--target".to_owned(),
        "android".to_owned(),
        "--apk".to_owned(),
        apk.display().to_string(),
        "--json".to_owned(),
    ];
    run_self_json(root, &args)?;
    Ok(())
}

fn update_cargo_version(root: &Path, version: &str) -> Result<bool> {
    let path = root.join("Cargo.toml");
    if !path.is_file() {
        return Ok(false);
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut output = String::new();
    let mut in_package = false;
    let mut changed = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed == "[package]" {
            in_package = true;
            output.push_str(line);
            output.push('\n');
            continue;
        }
        if in_package && !changed && trimmed.starts_with("version = \"") {
            let indent = &line[..line.len() - trimmed.len()];
            output.push_str(indent);
            output.push_str(&format!("version = \"{version}\""));
            output.push('\n');
            changed = true;
            in_package = false;
            continue;
        }
        if in_package && trimmed.starts_with('[') {
            in_package = false;
        }
        output.push_str(line);
        output.push('\n');
    }
    if changed {
        std::fs::write(&path, output).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(changed)
}

fn push_release(root: &Path, tag: Option<&str>) -> Result<()> {
    let remotes = git(root, &["remote"])?;
    if remotes.trim().is_empty() {
        bail!("no git remote configured; add one before pushing");
    }
    git(root, &["push", "origin", "HEAD"])?;
    if let Some(tag) = tag {
        git(root, &["push", "origin", tag])?;
        eprintln!("Pushed HEAD and {tag}");
    } else {
        eprintln!("Pushed HEAD");
    }
    Ok(())
}

fn bump_version(version: &str, bump: Bump) -> Result<String> {
    let mut parts = version.split('.');
    let major: u64 = parts
        .next()
        .and_then(|part| part.parse().ok())
        .ok_or_else(|| anyhow::anyhow!("app.version '{version}' is not semver"))?;
    let minor: u64 = parts
        .next()
        .and_then(|part| part.parse().ok())
        .ok_or_else(|| anyhow::anyhow!("app.version '{version}' is not semver"))?;
    let patch: u64 = parts
        .next()
        .and_then(|part| part.parse().ok())
        .ok_or_else(|| anyhow::anyhow!("app.version '{version}' is not semver"))?;
    let (major, minor, patch) = match bump {
        Bump::Major => (major + 1, 0, 0),
        Bump::Minor => (major, minor + 1, 0),
        Bump::Patch => (major, minor, patch + 1),
    };
    Ok(format!("{major}.{minor}.{patch}"))
}

fn release_notes(root: &Path) -> Result<String> {
    let last = git(root, &["describe", "--tags", "--abbrev=0"])
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let range = last
        .map(|tag| format!("{tag}..HEAD"))
        .unwrap_or_else(|| "HEAD".to_owned());
    let log = git(root, &["log", &range, "--pretty=format:%s"]).unwrap_or_default();

    let mut features = Vec::new();
    let mut fixes = Vec::new();
    let mut other = Vec::new();
    for line in log.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line.starts_with("feat") {
            features.push(line);
        } else if line.starts_with("fix") || line.starts_with("perf") {
            fixes.push(line);
        } else {
            other.push(line);
        }
    }

    let mut notes = String::from("## Changes\n");
    for (title, entries) in [
        ("### Features", features),
        ("### Fixes", fixes),
        ("### Other", other),
    ] {
        if entries.is_empty() {
            continue;
        }
        notes.push_str(title);
        notes.push('\n');
        for entry in entries {
            notes.push_str("- ");
            notes.push_str(entry);
            notes.push('\n');
        }
    }
    if notes == "## Changes\n" {
        notes.push_str("- No changes recorded.\n");
    }
    Ok(notes)
}

/// Drop installer formats whose external tool is missing: a release must not fail because the
/// machine has no WiX, and the plan has to match what packaging actually produces.
fn available_formats(formats: &[String]) -> Vec<String> {
    formats
        .iter()
        .filter(|format| match format.as_str() {
            "nsis" => winia_pack::nsis::find_makensis().is_ok(),
            "msi" => winia_pack::msi::wix_available(),
            _ => true,
        })
        .cloned()
        .collect()
}

/// Formats for one target, taken from the matching section of winia.toml.
fn formats_for(config: &WiniaConfig, target: &str) -> Vec<String> {
    match target {
        "windows" => config.package.windows.formats.clone(),
        "macos" => config.package.macos.formats.clone(),
        "linux" => config.package.linux.formats.clone(),
        _ => config.package.android.formats.clone(),
    }
}

fn planned_artifacts(
    stem: &str,
    version: &str,
    target: &str,
    arch: &str,
    formats: &[String],
    out_dir: &Path,
) -> Vec<PathBuf> {
    if target != "android" {
        return available_formats(formats)
            .iter()
            .filter_map(|format| {
                package::release_artifact_path(stem, version, target, arch, format, out_dir)
            })
            .collect();
    }
    let mut paths = Vec::new();
    if formats.iter().any(|format| format == "apk") {
        paths.push(out_dir.join(format!("{stem}-{version}-android-universal.apk")));
    }
    if formats.iter().any(|format| format == "aab") {
        paths.push(out_dir.join(format!("{stem}-{version}-android.aab")));
    }
    paths
}

/// A file counts as produced by this run when it is new or was rewritten after `started`. The
/// two-second slack covers filesystems with coarse timestamps (FAT), where a freshly written file
/// can still report a time slightly before the call that created it.
fn is_fresh(path: &Path, started: std::time::SystemTime) -> bool {
    let Ok(modified) = std::fs::metadata(path).and_then(|meta| meta.modified()) else {
        return false;
    };
    modified + std::time::Duration::from_secs(2) >= started
}

fn version_artifacts(dist: &Path, stem: &str, version: &str) -> Result<Vec<PathBuf>> {
    let prefix = format!("{stem}-{version}-");
    let mut artifacts: Vec<PathBuf> = std::fs::read_dir(dist)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .map(|name| {
                            path.is_file() && name.starts_with(&prefix) && is_release_file(path)
                        })
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    artifacts.sort();
    Ok(artifacts)
}

fn validate_signing_environment() -> Result<()> {
    if !keystore_configured() {
        return Ok(());
    }
    let raw = std::env::var_os("WINIA_ANDROID_KEYSTORE").expect("checked above");
    let keystore = PathBuf::from(raw);
    let keystore = if keystore.is_absolute() {
        keystore
    } else {
        std::env::current_dir()
            .context("reading the current directory")?
            .join(keystore)
    };
    if !keystore.is_file() {
        bail!("keystore not found: {}", keystore.display());
    }
    if std::env::var_os("WINIA_ANDROID_KEY_ALIAS")
        .filter(|value| !value.is_empty())
        .is_none()
    {
        bail!("WINIA_ANDROID_KEY_ALIAS is not set");
    }
    if std::env::var_os("WINIA_ANDROID_KEYSTORE_PASSWORD")
        .filter(|value| !value.is_empty())
        .is_none()
    {
        bail!("WINIA_ANDROID_KEYSTORE_PASSWORD is not set");
    }
    Ok(())
}

fn preflight_github(root: &Path, tag: &str) -> Result<()> {
    if !command_exists("gh") {
        bail!("GitHub release upload requires the gh CLI on PATH");
    }
    let auth = Command::new("gh")
        .args(["auth", "status"])
        .current_dir(root)
        .output()
        .context("running gh auth status")?;
    if !auth.status.success() {
        bail!("gh is not authenticated; run 'gh auth login'");
    }
    let remotes = git(root, &["remote"])?;
    if remotes.trim().is_empty() {
        bail!("no git remote configured; add one before --track github");
    }
    let output = Command::new("gh")
        .args(["release", "view", tag])
        .current_dir(root)
        .output()
        .context("running gh release view")?;
    if output.status.success() {
        bail!("GitHub release {tag} already exists");
    }
    Ok(())
}

/// File artifacts a release can publish; directories such as the macOS .app bundle are skipped.
fn is_release_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    matches!(
        extension(path),
        Some("apk" | "aab" | "zip" | "exe" | "msi" | "deb" | "dmg")
    ) || name.ends_with(".tar.gz")
}

fn is_android_artifact(path: &Path) -> bool {
    matches!(extension(path), Some("apk" | "aab"))
}

fn select_release_artifacts(artifacts: &[PathBuf]) -> Vec<PathBuf> {
    let mut selected: Vec<PathBuf> = artifacts
        .iter()
        .filter(|path| !is_android_artifact(path))
        .cloned()
        .collect();
    let unsigned_apk = artifacts.iter().find(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.ends_with("-universal.apk") && !name.ends_with("-signed.apk"))
            .unwrap_or(false)
    });
    if let Some(unsigned) = unsigned_apk {
        let expected = signed_path(unsigned);
        let chosen = artifacts
            .iter()
            .find(|path| path.as_path() == expected.as_path())
            .unwrap_or(unsigned);
        selected.push(chosen.clone());
    }
    selected.extend(
        artifacts
            .iter()
            .filter(|path| extension(path) == Some("aab"))
            .cloned(),
    );
    selected.sort();
    selected.dedup();
    selected
}

/// Rewrite dist/SHA256SUMS for exactly the artifacts that are published.
fn write_release_checksums(dist: &Path, artifacts: &[PathBuf]) -> Result<Option<PathBuf>> {
    let mut text = String::new();
    for path in artifacts {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        text.push_str(&format!("{:x}  {name}\n", Sha256::digest(&bytes)));
    }
    if text.is_empty() {
        return Ok(None);
    }
    let destination = dist.join("SHA256SUMS");
    std::fs::write(&destination, text)
        .with_context(|| format!("writing {}", destination.display()))?;
    Ok(Some(destination))
}

/// A macOS signing identity is configured (an ad-hoc "-" counts).
fn macos_identity_configured() -> bool {
    std::env::var_os("WINIA_MACOS_IDENTITY")
        .filter(|value| !value.is_empty())
        .is_some()
}

/// A Windows certificate is available for Authenticode signing.
fn windows_cert_configured() -> bool {
    ["WINIA_WINDOWS_CERT", "WINIA_WINDOWS_CERT_SHA1"]
        .iter()
        .any(|key| {
            std::env::var_os(key)
                .filter(|value| !value.is_empty())
                .is_some()
        })
}

fn keystore_configured() -> bool {
    std::env::var_os("WINIA_ANDROID_KEYSTORE")
        .filter(|value| !value.is_empty())
        .is_some()
}

fn upload_github(root: &Path, tag: &str, notes: &str, artifacts: &[PathBuf]) -> Result<()> {
    if !command_exists("gh") {
        bail!("GitHub release upload requires the gh CLI on PATH");
    }
    if git_status(root, &["remote"])? {
        let remotes = git(root, &["remote"])?;
        if remotes.trim().is_empty() {
            bail!("no git remote configured; add one before --track github");
        }
    }
    let notes_path = root.join("dist").join("release-notes.md");
    std::fs::create_dir_all(root.join("dist"))?;
    std::fs::write(&notes_path, notes)
        .with_context(|| format!("writing {}", notes_path.display()))?;
    let mut command = Command::new("gh");
    command
        .current_dir(root)
        .args(["release", "create", tag, "--title", tag, "--notes-file"])
        .arg(&notes_path);
    for artifact in artifacts {
        command.arg(artifact);
    }
    let output = command.output().context("running gh release create")?;
    if !output.status.success() {
        bail!(
            "gh release create failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    eprintln!("Published GitHub release {tag}");
    Ok(())
}

fn command_exists(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn write_manifest(dist: &Path, version: &str, tag: &str, artifacts: &[PathBuf]) -> Result<PathBuf> {
    let entries: Vec<_> = artifacts
        .iter()
        .map(|path| {
            let bytes = std::fs::metadata(path)
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            serde_json::json!({
                "name": path.file_name().and_then(|name| name.to_str()).unwrap_or_default(),
                "bytes": bytes,
            })
        })
        .collect();
    let value = serde_json::json!({
        "version": version,
        "tag": tag,
        "artifacts": entries,
    });
    let path = dist.join("latest.json");
    std::fs::write(&path, serde_json::to_string_pretty(&value)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .context("running git")?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn git_status(root: &Path, args: &[&str]) -> Result<bool> {
    Ok(Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false))
}

fn signed_path(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "app".to_owned());
    input.with_file_name(format!("{stem}-signed.apk"))
}

fn extension(path: &Path) -> Option<&str> {
    path.extension().and_then(|extension| extension.to_str())
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
    fn macos_bundles_and_images_are_publishable() {
        assert!(is_release_file(Path::new(
            "dist/app-0.1.0-macos-x86_64.dmg"
        )));
        assert!(is_release_file(Path::new(
            "dist/app-0.1.0-macos-x86_64.zip"
        )));
        // A bundle is a directory and cannot be an upload asset.
        assert!(!is_release_file(Path::new(
            "dist/app-0.1.0-macos-x86_64.app"
        )));
    }

    #[test]
    fn freshness_requires_a_file_written_by_this_run() {
        let dir = std::env::temp_dir().join(format!("winia-release-fresh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("artifact.zip");
        std::fs::write(&file, b"x").unwrap();
        let started = std::time::SystemTime::now();
        assert!(is_fresh(&file, started));
        assert!(!is_fresh(&dir.join("missing.zip"), started));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn keeps_desktop_artifacts_and_prefers_the_signed_apk() {
        let artifacts = vec![
            PathBuf::from("dist/app-1.0.0-windows-x86_64.zip"),
            PathBuf::from("dist/app-1.0.0-windows-x86_64-setup.exe"),
            PathBuf::from("dist/app-1.0.0-linux-x86_64.tar.gz"),
            PathBuf::from("dist/app-1.0.0-android-universal.apk"),
            PathBuf::from("dist/app-1.0.0-android-universal-signed.apk"),
            PathBuf::from("dist/app-1.0.0-android.aab"),
        ];
        let selected = select_release_artifacts(&artifacts);
        let names: Vec<&str> = selected
            .iter()
            .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
            .collect();
        assert!(names.contains(&"app-1.0.0-windows-x86_64.zip"));
        assert!(names.contains(&"app-1.0.0-linux-x86_64.tar.gz"));
        assert!(names.contains(&"app-1.0.0-android-universal-signed.apk"));
        assert!(!names.contains(&"app-1.0.0-android-universal.apk"));
        assert!(names.contains(&"app-1.0.0-android.aab"));
    }

    #[test]
    fn recognises_release_files() {
        assert!(is_release_file(Path::new(
            "dist/app-1.0.0-linux-x86_64.tar.gz"
        )));
        assert!(is_release_file(Path::new(
            "dist/app-1.0.0-windows-x86_64.msi"
        )));
        assert!(!is_release_file(Path::new("dist/app.app")));
        assert!(!is_release_file(Path::new("dist/SHA256SUMS")));
    }

    #[test]
    fn bumps_semver_components() {
        assert_eq!(bump_version("1.2.3", Bump::Patch).unwrap(), "1.2.4");
        assert_eq!(bump_version("1.2.3", Bump::Minor).unwrap(), "1.3.0");
        assert_eq!(bump_version("1.2.3", Bump::Major).unwrap(), "2.0.0");
        assert!(bump_version("1.2", Bump::Patch).is_err());
    }
}
