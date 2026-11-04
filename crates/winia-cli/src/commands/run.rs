//! 'winia run' — build and launch the application.
//!
//! Desktop targets build the host executable and wait for it to exit. Android
//! targets pick the connected device, read its primary ABI, build only that ABI,
//! install the APK, and start the activity. Arguments after `--` are forwarded
//! to the desktop application; on Android they become string intent extras
//! (`winia.args` and `winia.argN`).

use crate::cli::{BuildArgs, RunArgs};
use crate::commands::build;
use crate::config::WiniaConfig;
use crate::platform::android::Abi;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

pub fn run(args: RunArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("reading the current directory")?;
    let config_path = crate::config::find_config(&cwd)
        .ok_or_else(|| anyhow::anyhow!("winia.toml not found; run this inside a Winia project"))?;
    let root = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let config = WiniaConfig::load(&config_path)?;
    config.validate()?;

    let target = args
        .target
        .clone()
        .unwrap_or_else(|| build::host_target().to_owned());
    if target == "android" {
        run_android(&root, &config, &args)
    } else if target == build::host_target() {
        run_desktop(&root, &config, &args, &target)
    } else {
        bail!(
            "running the {target} target on {} is not supported yet",
            build::host_target()
        )
    }
}

fn build_args(target: &str, args: &RunArgs, abis: Vec<String>) -> BuildArgs {
    BuildArgs {
        target: vec![target.to_owned()],
        release: args.release,
        abi: abis,
        features: args.features.clone(),
        json: false,
        out_dir: None,
    }
}

fn run_desktop(root: &Path, config: &WiniaConfig, args: &RunArgs, target: &str) -> Result<()> {
    let built = build::build_all(root, config, &build_args(target, args, Vec::new()))?;
    let executable = artifacts(&built)
        .into_iter()
        .find(|artifact| extension(&artifact.path) != Some("apk"))
        .ok_or_else(|| anyhow::anyhow!("the build produced no executable"))?;
    println!("Running {}", executable.path.display());
    let status = Command::new(&executable.path)
        .args(&args.args)
        .current_dir(root)
        .status()
        .with_context(|| format!("running {}", executable.path.display()))?;
    if !status.success() {
        bail!("application exited with {status}");
    }
    Ok(())
}

fn run_android(root: &Path, config: &WiniaConfig, args: &RunArgs) -> Result<()> {
    let device = select_device(args.device.as_deref())?;
    let abi = device_abi(&device)?;
    println!("Device {device} uses {}", abi.as_str());

    let built = build::build_all(
        root,
        config,
        &build_args("android", args, vec![abi.as_str().to_owned()]),
    )?;
    let apk = artifacts(&built)
        .into_iter()
        .find(|artifact| extension(&artifact.path) == Some("apk"))
        .ok_or_else(|| anyhow::anyhow!("the build produced no APK"))?;
    install(&device, &apk.path)?;

    let package = if config.android.package.is_empty() {
        config.app.id.clone()
    } else {
        config.android.package.clone()
    };
    let component = activity_component(root, &package);
    launch(&device, &package, &component, &args.args)?;
    Ok(())
}

pub(crate) fn artifacts(built: &[(String, Vec<build::Artifact>)]) -> Vec<&build::Artifact> {
    built
        .iter()
        .flat_map(|(_, artifacts)| artifacts.iter())
        .collect()
}

pub(crate) fn extension(path: &Path) -> Option<&str> {
    path.extension().and_then(|extension| extension.to_str())
}

fn adb(args: &[&str]) -> Result<String> {
    let mut command = Command::new(adb_program());
    command.args(args);
    let output = output_with_timeout(command, Duration::from_secs(15))?;
    if !output.status.success() {
        bail!(
            "adb {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn adb_program() -> PathBuf {
    if Command::new("adb")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
    {
        return PathBuf::from("adb");
    }
    for variable in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(value) = std::env::var_os(variable).filter(|value| !value.is_empty()) {
            let candidate = PathBuf::from(value)
                .join("platform-tools")
                .join(if cfg!(windows) { "adb.exe" } else { "adb" });
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from("adb")
}

fn output_with_timeout(mut command: Command, timeout: Duration) -> Result<Output> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting adb")?;
    let started = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            bail!("adb command timed out after {}s", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    child.wait_with_output().context("collecting adb output")
}

pub(crate) fn select_device(requested: Option<&str>) -> Result<String> {
    if let Some(device) = requested.filter(|device| !device.is_empty()) {
        let mut command = Command::new(adb_program());
        command.args(["-s", device, "get-state"]);
        let state = output_with_timeout(command, Duration::from_secs(10))
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
        if state.as_deref() == Some("device") {
            return Ok(device.to_owned());
        }
        let online = parse_devices(&adb(&["devices"])?);
        let online = if online.is_empty() {
            "none".to_owned()
        } else {
            online.join(", ")
        };
        bail!("device '{device}' is not connected; online devices: {online}");
    }
    let output = adb(&["devices"])?;
    let devices = parse_devices(&output);
    match devices.len() {
        0 => bail!("no Android device or emulator connected; start one or pass --device <serial>"),
        1 => Ok(devices.into_iter().next().expect("length checked")),
        _ => bail!(
            "multiple devices connected: {}; pass --device <serial>",
            devices.join(", ")
        ),
    }
}

fn parse_devices(output: &str) -> Vec<String> {
    output
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let serial = fields.next()?;
            let state = fields.next()?;
            (state == "device").then(|| serial.to_owned())
        })
        .collect()
}

pub(crate) fn device_abi(device: &str) -> Result<Abi> {
    let output = adb(&["-s", device, "shell", "getprop", "ro.product.cpu.abi"])?;
    let raw = output.lines().next().unwrap_or_default().trim();
    Abi::parse(raw).with_context(|| format!("device {device} reports unsupported ABI '{raw}'"))
}

pub(crate) fn install(device: &str, apk: &Path) -> Result<()> {
    println!("Installing {}", apk.display());
    let mut command = Command::new(adb_program());
    command.args(["-s", device, "install", "-r"]).arg(apk);
    let output = output_with_timeout(command, Duration::from_secs(180))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success() || !text.contains("Success") {
        bail!("adb install failed for {}:\n{}", apk.display(), text.trim());
    }
    Ok(())
}

pub(crate) fn launch(device: &str, package: &str, component: &str, args: &[String]) -> Result<()> {
    let mut stop = Command::new(adb_program());
    stop.args(["-s", device, "shell", "am", "force-stop", package]);
    let _ = output_with_timeout(stop, Duration::from_secs(10));

    let mut start = Command::new(adb_program());
    start.args(["-s", device, "shell", "am", "start", "-W", "-n", component]);
    if !args.is_empty() {
        start.arg("--es").arg("winia.args").arg(args.join(" "));
        for (index, value) in args.iter().enumerate() {
            start
                .arg("--es")
                .arg(format!("winia.arg{index}"))
                .arg(value);
        }
    }
    let output = output_with_timeout(start, Duration::from_secs(30))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success() || text.contains("Error") {
        bail!("could not start {component}:\n{}", text.trim());
    }
    println!("Launched {component} on {device}");
    Ok(())
}

/// Resolve the launcher activity from the generated manifest, falling back to
/// the template's `.MainActivity`.
pub(crate) fn activity_component(root: &Path, package: &str) -> String {
    let manifest = root.join("android/app/src/main/AndroidManifest.xml");
    if let Ok(text) = std::fs::read_to_string(&manifest) {
        if let Some(name) = first_activity_name(&text) {
            return if name.starts_with('.') {
                format!("{package}/{name}")
            } else if name.starts_with(package) {
                name
            } else {
                format!("{package}/{name}")
            };
        }
    }
    format!("{package}/.MainActivity")
}

fn first_activity_name(manifest: &str) -> Option<String> {
    let mut rest = manifest;
    while let Some(start) = rest.find("<activity") {
        rest = &rest[start..];
        let end = rest.find('>')?;
        let element = &rest[..end];
        let marker = "android:name=\"";
        if let Some(name_start) = element.find(marker) {
            let value = &element[name_start + marker.len()..];
            if let Some(quote) = value.find('"') {
                return Some(value[..quote].to_owned());
            }
        }
        rest = &rest[end + 1..];
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_activity_name_from_the_manifest() {
        let manifest = r#"<manifest><application>
            <activity android:name=".Launcher" android:exported="true" />
        </application></manifest>"#;
        assert_eq!(first_activity_name(manifest), Some(".Launcher".to_owned()));
        assert!(first_activity_name("<manifest />").is_none());
    }

    #[test]
    fn parses_only_ready_devices() {
        let output = "List of devices attached\n\
            emulator-5554\tdevice\n\
            192.0.0.172:5555\tdevice\n\
            10.0.0.2:5555\toffline\n\
            10.0.0.3:5555\tunauthorized\n";
        assert_eq!(
            parse_devices(output),
            vec!["emulator-5554", "192.0.0.172:5555"]
        );
    }
}
