//! 'winia clean' — remove build artifacts from a project.
//!
//! Only known project-relative paths are touched. Symlinks or paths that resolve
//! outside the project root are skipped. `--all` additionally removes the
//! project-local Gradle cache directories.

use crate::cli::CleanArgs;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

struct Target {
    label: &'static str,
    path: PathBuf,
}

struct ResultEntry {
    path: String,
    bytes: u64,
    status: &'static str,
    reason: Option<String>,
}

pub fn run(args: CleanArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("reading the current directory")?;
    let config_path = crate::config::find_config(&cwd)
        .ok_or_else(|| anyhow::anyhow!("winia.toml not found; run this inside a Winia project"))?;
    let root = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();

    let mut targets = vec![
        Target {
            label: "target",
            path: root.join("target"),
        },
        Target {
            label: "dist",
            path: root.join("dist"),
        },
        Target {
            label: "android/app/build",
            path: root.join("android/app/build"),
        },
        Target {
            label: "android/build",
            path: root.join("android/build"),
        },
    ];
    if args.all {
        targets.push(Target {
            label: "android/.gradle",
            path: root.join("android/.gradle"),
        });
    }

    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("resolving {}", root.display()))?;
    let mut entries = Vec::new();
    let mut total_bytes = 0u64;

    for target in targets {
        if !target.path.exists() {
            entries.push(ResultEntry {
                path: target.label.to_owned(),
                bytes: 0,
                status: "skipped",
                reason: Some("not found".to_owned()),
            });
            continue;
        }
        let canonical = match target.path.canonicalize() {
            Ok(path) => path,
            Err(error) => {
                entries.push(ResultEntry {
                    path: target.label.to_owned(),
                    bytes: 0,
                    status: "skipped",
                    reason: Some(format!("cannot resolve: {error}")),
                });
                continue;
            }
        };
        if !canonical.starts_with(&canonical_root) || canonical == canonical_root {
            entries.push(ResultEntry {
                path: target.label.to_owned(),
                bytes: 0,
                status: "skipped",
                reason: Some("resolves outside the project root".to_owned()),
            });
            continue;
        }

        let bytes = path_size(&target.path);
        if args.dry_run {
            entries.push(ResultEntry {
                path: target.label.to_owned(),
                bytes,
                status: "would-remove",
                reason: None,
            });
            total_bytes += bytes;
            continue;
        }
        match std::fs::remove_dir_all(&target.path) {
            Ok(()) => {
                entries.push(ResultEntry {
                    path: target.label.to_owned(),
                    bytes,
                    status: "removed",
                    reason: None,
                });
                total_bytes += bytes;
            }
            Err(error) => {
                entries.push(ResultEntry {
                    path: target.label.to_owned(),
                    bytes: 0,
                    status: "skipped",
                    reason: Some(format!("remove failed: {error}")),
                });
            }
        }
    }

    if args.json {
        let values: Vec<_> = entries
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "path": entry.path,
                    "bytes": entry.bytes,
                    "status": entry.status,
                    "reason": entry.reason,
                })
            })
            .collect();
        let value = serde_json::json!({
            "root": root,
            "dry_run": args.dry_run,
            "removed": values,
            "total_bytes": total_bytes,
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }

    for entry in &entries {
        match entry.status {
            "removed" => println!("Removed {} ({})", entry.path, format_bytes(entry.bytes)),
            "would-remove" => println!(
                "Would remove {} ({})",
                entry.path,
                format_bytes(entry.bytes)
            ),
            "skipped" => {
                if let Some(reason) = &entry.reason {
                    println!("Skipped {} ({reason})", entry.path);
                } else {
                    println!("Skipped {}", entry.path);
                }
            }
            _ => {}
        }
    }
    if entries.iter().all(|entry| entry.status == "skipped") {
        println!("Nothing to clean.");
    } else if !args.dry_run {
        println!(
            "Cleaned {} in {:.2}",
            format_bytes(total_bytes),
            root.display()
        );
    }
    Ok(())
}

fn path_size(path: &Path) -> u64 {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return 0,
    };
    if metadata.file_type().is_symlink() {
        return 0;
    }
    if metadata.is_file() {
        return metadata.len();
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| path_size(&entry.path()))
        .sum()
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes() {
        assert_eq!(format_bytes(512), "512.0 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
    }
}
