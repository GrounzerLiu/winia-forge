//! 'winia logcat' streams the device log, optionally filtered by tag, package or pid.

use crate::cli::LogcatArgs;
use crate::commands::run;
use anyhow::{Context, Result, bail};
use std::process::Command;

pub fn run(args: LogcatArgs) -> Result<()> {
    let device = run::select_device(args.device.as_deref())?;
    if args.clear {
        run::adb_on(&device, &["logcat", "-c"])?;
    }

    let mut arguments: Vec<String> = vec![
        "-s".to_owned(),
        device.clone(),
        "logcat".to_owned(),
        "-v".to_owned(),
        "time".to_owned(),
    ];

    if let Some(pid) = args.pid {
        arguments.push(format!("--pid={pid}"));
    } else if let Some(filter) = args.filter.as_deref().filter(|value| !value.is_empty()) {
        if filter.contains('.') {
            // Looks like a package: resolve it to a pid once so the stream stays cheap.
            let pid = run::adb_on(&device, &["shell", "pidof", "-s", filter])
                .map(|output| output.trim().to_owned())
                .unwrap_or_default();
            if pid.is_empty() {
                bail!("'{filter}' is not running on {device}");
            }
            arguments.push(format!("--pid={pid}"));
        } else {
            arguments.push("-s".to_owned());
            arguments.push(filter.to_owned());
        }
    }

    if let Some(level) = args.level.as_deref().filter(|value| !value.is_empty()) {
        let level = level.to_ascii_uppercase();
        if !matches!(level.as_str(), "V" | "D" | "I" | "W" | "E" | "F" | "S") {
            bail!("unknown log level '{level}'; expected V, D, I, W, E, F or S");
        }
        arguments.push(format!("*:{level}"));
    }

    let status = Command::new(run::adb_program())
        .args(&arguments)
        .status()
        .context("running adb logcat")?;
    match status.code() {
        Some(0) => Ok(()),
        // Ctrl+C or a closed pipe stops the stream; that is not a failure.
        Some(130) | Some(143) => Ok(()),
        None => Ok(()),
        _ => bail!("adb logcat exited with {status}"),
    }
}
