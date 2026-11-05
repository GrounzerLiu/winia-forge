//! 'winia emulators' lists and controls Android virtual devices.
//!
//! Listing reads 'emulator -list-avds' and cross-checks adb, so a running AVD shows its serial.
//! Starting spawns the emulator detached; stopping maps an AVD name back to its emulator serial
//! through 'adb emu avd name' and sends 'emu kill', which is the only portable way to stop one.

use crate::cli::{EmulatorsArgs, EmulatorsCommand};
use crate::commands::{devices, run};
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub fn run(args: EmulatorsArgs) -> Result<()> {
    match args.command {
        EmulatorsCommand::List => list(),
        EmulatorsCommand::Start { name, headless } => start(&name, headless),
        EmulatorsCommand::Stop { name } => stop(&name),
    }
}

fn list() -> Result<()> {
    let output = Command::new(emulator_program())
        .arg("-list-avds")
        .output()
        .context("running emulator -list-avds (is the Android SDK on this machine?)")?;
    if !output.status.success() {
        bail!(
            "emulator -list-avds failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let names = parse_avds(&String::from_utf8_lossy(&output.stdout));
    if names.is_empty() {
        println!("No AVDs found. Create one with the Android Studio device manager.");
        return Ok(());
    }
    let running = running_emulators()?;
    println!("{:<24} SERIAL", "AVD");
    for name in &names {
        let serial = running
            .iter()
            .find(|(avd, _)| avd == name)
            .map(|(_, serial)| serial.as_str())
            .unwrap_or("-");
        println!("{:<24} {}", name, serial);
    }
    Ok(())
}

fn start(name: &str, headless: bool) -> Result<()> {
    let mut command = Command::new(emulator_program());
    command.arg("-avd").arg(name);
    if headless {
        command
            .arg("-no-window")
            .arg("-no-audio")
            .arg("-no-boot-anim");
    }
    let child = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("starting the emulator for '{name}'"))?;
    println!(
        "Started {name} (pid {}). Use 'winia devices' to wait for it to come online.",
        child.id()
    );
    Ok(())
}

fn stop(name: &str) -> Result<()> {
    let running = running_emulators()?;
    let Some((_, serial)) = running.iter().find(|(avd, _)| avd == name) else {
        bail!("no running emulator for '{name}'");
    };
    run::adb_on(serial, &["emu", "kill"]).with_context(|| format!("stopping {serial}"))?;
    println!("Stopped {name} ({serial})");
    Ok(())
}

/// AVD names reported by the emulator, one per line.
pub fn parse_avds(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// (avd name, serial) for every online emulator.
fn running_emulators() -> Result<Vec<(String, String)>> {
    let output = run::adb(&["devices"])?;
    let mut running = Vec::new();
    for device in devices::parse(&output) {
        if device.state != "device" || !device.serial.starts_with("emulator-") {
            continue;
        }
        let Ok(name) = run::adb_on(&device.serial, &["emu", "avd", "name"]) else {
            continue;
        };
        let name = name.lines().next().unwrap_or("").trim().to_owned();
        if !name.is_empty() {
            running.push((name, device.serial));
        }
    }
    Ok(running)
}

/// The emulator binary from PATH, or the Android SDK.
fn emulator_program() -> PathBuf {
    let exe = if cfg!(windows) {
        "emulator.exe"
    } else {
        "emulator"
    };
    if Command::new(exe)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
    {
        return PathBuf::from(exe);
    }
    for variable in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(value) = std::env::var_os(variable).filter(|value| !value.is_empty()) {
            let candidate = PathBuf::from(value).join("emulator").join(exe);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from(exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_avd_names() {
        let output = "Pixel_7_API_34\nagnes36\n\n";
        assert_eq!(parse_avds(output), vec!["Pixel_7_API_34", "agnes36"]);
    }

    #[test]
    fn ignores_blank_output() {
        assert!(parse_avds("\n  \n").is_empty());
    }
}
