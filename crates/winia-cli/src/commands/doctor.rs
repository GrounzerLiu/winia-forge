//! 'winia doctor' — check the host toolchain and platform SDKs.
//!
//! The first version is read-only and intentionally conservative: it reports
//! what it can find and never claims an environment is complete when a tool is
//! missing. Auto-fix is planned.

use crate::cli::DoctorArgs;
use std::path::Path;
use std::process::Command;

pub fn run(args: DoctorArgs) -> Result<(), anyhow::Error> {
    let checks: Vec<Check> = vec![
        command_check("rustc", &["--version"]),
        command_check("cargo", &["--version"]),
        command_check("git", &["--version"]),
        command_check("adb", &["version"]),
        env_check("ANDROID_HOME"),
        env_check("ANDROID_SDK_ROOT"),
        env_check("ANDROID_NDK"),
        env_check("ANDROID_NDK_HOME"),
        env_check("JAVA_HOME"),
        config_check(),
    ];

    if args.json {
        let values: Vec<_> = checks
            .iter()
            .map(|check| {
                serde_json::json!({
                    "name": check.name,
                    "status": if check.ok { "found" } else { "missing" },
                    "detail": check.detail,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&values)?);
        return Ok(());
    }

    println!("Winia Forge environment check");
    if let Some(target) = &args.target {
        println!("Target focus: {target}");
    }
    println!();
    for check in &checks {
        let status = if check.ok { "found  " } else { "missing" };
        println!("  {:<18} {}  {}", check.name, status, check.detail);
    }
    println!();
    if args.fix {
        println!("Auto-fix is not implemented yet; install the missing tools manually.");
    } else {
        println!("Run with --fix once auto-fix lands.");
    }
    Ok(())
}

struct Check {
    name: &'static str,
    ok: bool,
    detail: String,
}

fn command_check(name: &'static str, args: &[&str]) -> Check {
    match Command::new(name).args(args).output() {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            let first = text.lines().next().unwrap_or("").trim().to_owned();
            Check {
                name,
                ok: true,
                detail: first,
            }
        }
        Ok(output) => {
            let text = String::from_utf8_lossy(&output.stderr);
            let first = text
                .lines()
                .next()
                .unwrap_or("command failed")
                .trim()
                .to_owned();
            Check {
                name,
                ok: false,
                detail: first,
            }
        }
        Err(error) => Check {
            name,
            ok: false,
            detail: error.to_string(),
        },
    }
}

fn env_check(name: &'static str) -> Check {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => Check {
            name,
            ok: true,
            detail: value,
        },
        _ => Check {
            name,
            ok: false,
            detail: "not set".to_owned(),
        },
    }
}

fn config_check() -> Check {
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    match crate::config::find_config(&cwd) {
        Some(path) => Check {
            name: "winia.toml",
            ok: true,
            detail: path.display().to_string(),
        },
        None => Check {
            name: "winia.toml",
            ok: false,
            detail: "not found in this directory tree".to_owned(),
        },
    }
}
