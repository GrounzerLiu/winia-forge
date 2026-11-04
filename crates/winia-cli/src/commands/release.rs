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
    if let Some(unsupported) = targets.iter().find(|target| target.as_str() != "android") {
        bail!("releasing {unsupported} is not implemented yet; only --target android is supported");
    }
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
    let formats = config.package.android.formats.clone();
    let planned = planned_artifacts(&stem, &version, &formats, &out_dir);
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
                "target": "android",
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
        config.save(&config_path)?;
        let mut files = vec!["winia.toml".to_owned()];
        if update_cargo_version(&root, &version)? {
            files.push("Cargo.toml".to_owned());
        }
        let mut add = vec!["add".to_owned()];
        add.extend(files);
        let add_args: Vec<&str> = add.iter().map(String::as_str).collect();
        git(&root, &add_args)?;
        git(&root, &["commit", "-m", &format!("chore(release): {tag}")])?;
    }

    if args.json {
        run_package_json(&root, &formats, &out_dir)?;
    } else {
        package::run(PackageArgs {
            target: vec!["android".to_owned()],
            format: formats.clone(),
            release: true,
            out_dir: Some(out_dir.clone()),
            split: None,
            json: false,
            dry_run: false,
            clean: false,
        })?;
    }

    let mut artifacts = version_artifacts(&out_dir, &stem, &version)?;
    if artifacts.is_empty() {
        bail!("packaging produced no artifacts for {version}");
    }

    if keystore_configured() {
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
    } else if !artifacts.is_empty() {
        eprintln!("warning: no Android keystore configured; release artifacts are unsigned");
    }
    let artifacts = select_release_artifacts(&artifacts);

    if !args.no_tag {
        git(&root, &["tag", "-a", &tag, "-m", &tag])?;
    }
    if push {
        push_release(&root, if args.no_tag { None } else { Some(&tag) })?;
    }

    if track == Some("github") {
        upload_github(&root, &tag, &notes, &artifacts)?;
    }

    write_manifest(&out_dir, &version, &tag, &artifacts)?;

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
            "target": "android",
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

fn run_package_json(root: &Path, formats: &[String], out_dir: &Path) -> Result<()> {
    let mut args = vec![
        "package".to_owned(),
        "--target".to_owned(),
        "android".to_owned(),
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

fn planned_artifacts(
    stem: &str,
    version: &str,
    formats: &[String],
    out_dir: &Path,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if formats.iter().any(|format| format == "apk") {
        paths.push(out_dir.join(format!("{stem}-{version}-android-universal.apk")));
    }
    if formats.iter().any(|format| format == "aab") {
        paths.push(out_dir.join(format!("{stem}-{version}-android.aab")));
    }
    paths
}

fn version_artifacts(dist: &Path, stem: &str, version: &str) -> Result<Vec<PathBuf>> {
    let prefix = format!("{stem}-{version}-android");
    let mut artifacts: Vec<PathBuf> = std::fs::read_dir(dist)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .map(|name| {
                            name.starts_with(&prefix)
                                && matches!(extension(path), Some("apk" | "aab"))
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

fn select_release_artifacts(artifacts: &[PathBuf]) -> Vec<PathBuf> {
    let mut selected = Vec::new();
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
    selected
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

fn write_manifest(dist: &Path, version: &str, tag: &str, artifacts: &[PathBuf]) -> Result<()> {
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
    Ok(())
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
    fn bumps_semver_components() {
        assert_eq!(bump_version("1.2.3", Bump::Patch).unwrap(), "1.2.4");
        assert_eq!(bump_version("1.2.3", Bump::Minor).unwrap(), "1.3.0");
        assert_eq!(bump_version("1.2.3", Bump::Major).unwrap(), "2.0.0");
        assert!(bump_version("1.2", Bump::Patch).is_err());
    }
}
