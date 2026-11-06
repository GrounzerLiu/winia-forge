//! macOS code signing through codesign.
//!
//! The identity comes from WINIA_MACOS_IDENTITY; the default is an ad-hoc signature ("-"), which
//! is enough for local testing but not for distribution. Entitlements are optional through
//! WINIA_MACOS_ENTITLEMENTS. Notarisation and stapling are not implemented yet.

use crate::cli::SignArgs;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

/// Sign the given bundles (or every .app in dist) with codesign.
pub fn run(root: &Path, cwd: &Path, args: &SignArgs) -> Result<()> {
    if !cfg!(target_os = "macos") && !args.dry_run {
        bail!("code signing needs codesign, which only exists on macOS; run this on a Mac");
    }
    let started = Instant::now();
    let identity = std::env::var("WINIA_MACOS_IDENTITY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "-".to_owned());
    let entitlements = std::env::var_os("WINIA_MACOS_ENTITLEMENTS").map(PathBuf::from);
    let bundles = resolve_bundles(root, cwd, args)?;
    if bundles.is_empty() {
        bail!(
            "no .app bundles to sign; run 'winia package --target macos --format app' first or pass --file"
        );
    }

    if args.dry_run {
        if !args.quiet {
            for bundle in &bundles {
                println!("Would sign {}", relative(root, bundle));
                println!("  {}", render(&identity, entitlements.as_deref(), bundle));
            }
        }
        return Ok(());
    }

    for bundle in &bundles {
        let mut command = Command::new("codesign");
        command
            .arg("--force")
            .arg("--deep")
            .arg("--sign")
            .arg(&identity);
        if identity != "-" {
            command.arg("--options").arg("runtime").arg("--timestamp");
        }
        if let Some(entitlements) = &entitlements {
            command.arg("--entitlements").arg(entitlements);
        }
        command.arg(bundle);
        let output = command
            .output()
            .context("running codesign (is Xcode installed?)")?;
        if !output.status.success() {
            bail!(
                "codesign failed for {}: {}",
                bundle.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        if !verify(bundle) {
            eprintln!(
                "note: {} is signed but codesign --verify reported a problem",
                relative(root, bundle)
            );
        }
    }

    if args.json {
        let values: Vec<_> = bundles
            .iter()
            .map(|path| serde_json::json!({ "path": relative(root, path) }))
            .collect();
        let value = serde_json::json!({
            "target": "macos",
            "identity": identity,
            "artifacts": values,
            "duration_ms": started.elapsed().as_millis(),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }

    if args.quiet {
        return Ok(());
    }
    for bundle in &bundles {
        println!("Signed {}", relative(root, bundle));
    }
    println!(
        "Signed {} bundle(s) with {identity} in {:.1}s",
        bundles.len(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

fn verify(bundle: &Path) -> bool {
    Command::new("codesign")
        .arg("--verify")
        .arg("--deep")
        .arg("--strict")
        .arg(bundle)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn render(identity: &str, entitlements: Option<&Path>, bundle: &Path) -> String {
    let mut parts = vec![
        "codesign".to_owned(),
        "--force".to_owned(),
        "--deep".to_owned(),
        "--sign".to_owned(),
        format!("\"{identity}\""),
    ];
    if identity != "-" {
        parts.push("--options".to_owned());
        parts.push("runtime".to_owned());
        parts.push("--timestamp".to_owned());
    }
    if let Some(entitlements) = entitlements {
        parts.push("--entitlements".to_owned());
        parts.push(format!("\"{}\"", entitlements.display()));
    }
    parts.push(format!("\"{}\"", bundle.display()));
    parts.join(" ")
}

/// Explicit --file entries, then --apk, otherwise every .app in dist.
fn resolve_bundles(root: &Path, cwd: &Path, args: &SignArgs) -> Result<Vec<PathBuf>> {
    let mut bundles = Vec::new();
    for path in &args.file {
        let path = if path.is_absolute() {
            path.clone()
        } else {
            cwd.join(path)
        };
        if !path.exists() {
            bail!("{} does not exist", path.display());
        }
        bundles.push(path);
    }
    if let Some(bundle) = &args.apk {
        let path = if bundle.is_absolute() {
            bundle.clone()
        } else {
            cwd.join(bundle)
        };
        if !path.exists() {
            bail!("{} does not exist", path.display());
        }
        bundles.push(path);
    }
    if bundles.is_empty() {
        let dist = root.join("dist");
        if dist.is_dir() {
            for entry in
                std::fs::read_dir(&dist).with_context(|| format!("reading {}", dist.display()))?
            {
                let path = entry?.path();
                if path.extension().and_then(|value| value.to_str()) == Some("app") {
                    bundles.push(path);
                }
            }
            bundles.sort();
        }
    }
    bundles.dedup();
    Ok(bundles)
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_an_ad_hoc_command_without_timestamping() {
        let command = render("-", None, Path::new("App.app"));
        assert!(command.contains("--sign \"-\""));
        assert!(!command.contains("--timestamp"));
    }

    #[test]
    fn renders_a_developer_id_command_with_timestamping() {
        let command = render(
            "Developer ID Application: X (TEAM)",
            Some(Path::new("entitlements.plist")),
            Path::new("App.app"),
        );
        assert!(command.contains("--timestamp"));
        assert!(command.contains("--entitlements"));
    }
}
